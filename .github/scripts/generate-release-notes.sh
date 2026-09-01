#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "Usage: $0 <tag> <repository-url> <output-file>" >&2
  exit 2
fi

tag="$1"
repository_url="${2%/}"
output_file="$3"
previous_tag="$(git tag --merged "${tag}^{}" --list 'v*' --sort=-version:refname | grep -Fxv "${tag}" | head -n 1 || true)"

cat > "${output_file}" <<EOF
## 安装与使用

Linux x86_64 一键安装：

\`\`\`bash
curl --proto '=https' --tlsv1.2 -LsSf ${repository_url}/raw/${tag}/install.sh | sh
\`\`\`

Windows x86_64 PowerShell 一键安装：

\`\`\`powershell
irm ${repository_url}/raw/${tag}/install.ps1 | iex
\`\`\`

下方 **Assets** 同时提供 Linux x86_64、macOS Intel、macOS Apple Silicon 的 \`.tar.gz\` 和 Windows x86_64 的 \`.zip\`。安装脚本和 \`trepo update\` 都会使用 \`SHA256SUMS\` 校验版本化产物。

\`\`\`bash
trepo /path/to/git-or-repo-workspace
trepo -zh /path/to/git-or-repo-workspace
trepo doctor /path/to/git-or-repo-workspace
trepo update --check
trepo update
\`\`\`

运行 Repo 工作区模式需要安装 Google/Android \`repo\`；所有模式都需要 Git。Windows 原生发布支持 Git 仓库模式，Repo 模式取决于外部 \`repo\` 工具是否可用。

## 提交改动

EOF

if [[ -n "${previous_tag}" ]]; then
  echo "[查看 ${previous_tag}...${tag} 的完整差异](${repository_url}/compare/${previous_tag}...${tag})" >> "${output_file}"
  echo >> "${output_file}"
  range="${previous_tag}..${tag}"
else
  echo "首次发布，以下列出截至 ${tag} 的提交。" >> "${output_file}"
  echo >> "${output_file}"
  range="${tag}"
fi

git log --no-merges --pretty="- %s ([\`%h\`](${repository_url}/commit/%H))" "${range}" >> "${output_file}"
