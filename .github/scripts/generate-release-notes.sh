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

1. 在下方 **Assets** 下载与系统匹配的压缩包：
   - Linux x86_64: \`trepo-${tag}-linux-x86_64.tar.gz\`
   - macOS Intel: \`trepo-${tag}-macos-x86_64.tar.gz\`
   - macOS Apple Silicon: \`trepo-${tag}-macos-aarch64.tar.gz\`
2. 解压并安装：

\`\`\`bash
tar -xzf trepo-${tag}-<platform>.tar.gz
install -m 0755 trepo-${tag}-<platform>/trepo ~/.local/bin/trepo
\`\`\`

3. 启动或诊断工作区：

\`\`\`bash
trepo /path/to/git-or-repo-workspace
trepo -zh /path/to/git-or-repo-workspace
trepo doctor /path/to/git-or-repo-workspace
\`\`\`

运行 Repo 工作区模式需要安装 Google/Android \`repo\`；所有模式都需要 Git。可用 \`SHA256SUMS\` 校验下载产物。

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
