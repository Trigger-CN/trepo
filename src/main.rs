use std::io::{self, IsTerminal, Stdout};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tracing_subscriber::EnvFilter;

use trepo::adapters::repo;
use trepo::app::state::{App, Screen};
use trepo::i18n::Language;
use trepo::services::discovery;

#[derive(Debug, Parser)]
#[command(name = "trepo", version, about)]
struct Cli {
    #[arg(default_value = ".")]
    path: PathBuf,

    #[arg(long, default_value_t = default_concurrency())]
    scan_concurrency: usize,

    #[arg(long)]
    log_file: Option<PathBuf>,

    #[arg(long, conflicts_with = "en")]
    zh: bool,

    #[arg(long, conflicts_with = "zh")]
    en: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Diagnose Git, Repo and workspace discovery without starting the TUI.
    Doctor {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse_from(normalize_language_args(std::env::args_os()));
    init_logging(cli.log_file.as_deref())?;

    if let Some(Commands::Doctor { path }) = cli.command {
        return doctor(&path).await;
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("trepo requires an interactive terminal; use `trepo doctor` for diagnostics");
    }

    let workspace = discovery::discover(&cli.path).await?;
    let language = if cli.zh { Language::Zh } else { Language::En };
    let mut app = App::new_with_language(workspace, cli.scan_concurrency, language);
    app.refresh();
    run_tui(&mut app).await
}

fn normalize_language_args(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Vec<std::ffi::OsString> {
    args.into_iter()
        .map(|arg| match arg.to_str() {
            Some("-zh") => std::ffi::OsString::from("--zh"),
            Some("-en") => std::ffi::OsString::from("--en"),
            _ => arg,
        })
        .collect()
}

fn init_logging(log_file: Option<&Path>) -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    if let Some(path) = log_file {
        let file = std::fs::File::create(path)
            .with_context(|| format!("failed to create log file {}", path.display()))?;
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(file)
            .with_ansi(false)
            .try_init()
            .ok();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(io::stderr)
            .try_init()
            .ok();
    }
    Ok(())
}

async fn doctor(path: &Path) -> Result<()> {
    println!("trepo {}", env!("CARGO_PKG_VERSION"));
    println!("input: {}", path.display());
    println!(
        "terminal stdin/stdout: {}/{}",
        io::stdin().is_terminal(),
        io::stdout().is_terminal()
    );

    match command_version("git", &["--version"]).await {
        Ok(version) => println!("git: {version}"),
        Err(error) => println!("git: unavailable ({error})"),
    }
    match repo::version().await {
        Ok(version) => println!("repo: {version}"),
        Err(error) => println!("repo: unavailable ({error})"),
    }
    match discovery::discover(path).await {
        Ok(workspace) => {
            println!("workspace: {:?}", workspace.kind);
            println!("root: {}", workspace.root.display());
            println!("projects: {}", workspace.projects.len());
            for project in workspace.projects.iter().take(5) {
                println!("  {} [{}]", project.relative_path.display(), project.name);
            }
            if workspace.projects.len() > 5 {
                println!("  ... {} more", workspace.projects.len() - 5);
            }
            Ok(())
        }
        Err(error) => anyhow::bail!("workspace discovery failed: {error}"),
    }
}

async fn command_version(program: &str, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to run {program}"))?;
    if !output.status.success() {
        anyhow::bail!("{program} exited with {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable terminal raw mode")?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableBracketedPaste) {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enter alternate screen");
        }
        let backend = CrosstermBackend::new(stdout);
        let terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = disable_raw_mode();
                let mut stdout = io::stdout();
                let _ = execute!(stdout, DisableBracketedPaste, LeaveAlternateScreen);
                return Err(error).context("failed to initialize terminal");
            }
        };
        Ok(Self { terminal })
    }

    fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

async fn run_tui(app: &mut App) -> Result<()> {
    let mut terminal = TerminalGuard::enter()?;
    loop {
        terminal
            .terminal()
            .draw(|frame| trepo::ui::render(frame, app))
            .context("failed to draw terminal UI")?;

        drain_background_messages(app);
        if app.should_quit {
            break;
        }

        if event::poll(Duration::from_millis(50)).context("failed to poll terminal input")? {
            match event::read().context("failed to read terminal input")? {
                Event::Key(key) if key.kind == KeyEventKind::Press => handle_key(app, key),
                Event::Paste(text) => {
                    app.edit_commit_message(trepo::app::state::CommitInput::Text(text))
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        drain_background_messages(app);
    }
    Ok(())
}

fn drain_background_messages(app: &mut App) {
    while let Ok(result) = app.scan_rx.try_recv() {
        app.apply_scan(result);
    }
    while let Ok(result) = app.graph_rx.try_recv() {
        app.apply_graph(result);
    }
    while let Ok(result) = app.changes_rx.try_recv() {
        app.apply_changes(result);
    }
    while let Ok(result) = app.preview_rx.try_recv() {
        app.apply_preview(result);
    }
    while let Ok(result) = app.batch_prepare_rx.try_recv() {
        app.apply_batch_prepare(result);
    }
    while let Ok(result) = app.operation_rx.try_recv() {
        app.apply_operation(result);
    }
    while let Ok(result) = app.commit_rx.try_recv() {
        app.apply_commit(result);
    }
    while let Ok(result) = app.graph_commit_rx.try_recv() {
        app.apply_graph_commit(result);
    }
    while let Ok(result) = app.repository_rx.try_recv() {
        app.apply_repository_load(result);
    }
    while let Ok(result) = app.repository_action_rx.try_recv() {
        app.apply_repository_action(result);
    }
    while let Ok(result) = app.workspace_git_prepare_rx.try_recv() {
        app.apply_workspace_git_prepare(result);
    }
    while let Ok(event) = app.workspace_git_rx.try_recv() {
        app.apply_workspace_git(event);
    }
    while let Ok(event) = app.repo_batch_rx.try_recv() {
        app.apply_repo_batch(event);
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    if app.screen == Screen::Workspace && app.workspace_git_overlay_active() {
        let pending = app.workspace_git.pending.is_some();
        let running = app
            .workspace_git
            .task
            .as_ref()
            .is_some_and(|task| task.running);
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') if pending => app.confirm_workspace_git(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc if pending => {
                app.confirm_workspace_git(false);
                app.close_workspace_git();
            }
            KeyCode::Esc if !running => app.close_workspace_git(),
            KeyCode::Down | KeyCode::Char('j') => app.scroll_workspace_git(1),
            KeyCode::Up | KeyCode::Char('k') => app.scroll_workspace_git(-1),
            KeyCode::PageDown => app.scroll_workspace_git(10),
            KeyCode::PageUp => app.scroll_workspace_git(-10),
            _ => {}
        }
        return;
    }
    if app.screen == Screen::Workspace && app.repo_batch_overlay_active() {
        let menu = app.repo_batch.action_menu;
        let form = app.repo_batch.form.is_some();
        let pending = app.repo_batch.pending.is_some();
        let running = app
            .repo_batch
            .task
            .as_ref()
            .is_some_and(|task| task.running);
        match key.code {
            KeyCode::Esc if !running => app.close_repo_batch_overlay(),
            KeyCode::Char('y') | KeyCode::Char('Y') if pending => app.confirm_repo_batch(true),
            KeyCode::Char('n') | KeyCode::Char('N') if pending => app.confirm_repo_batch(false),
            KeyCode::Enter if form => app.submit_repo_batch_form(),
            KeyCode::Enter if menu => app.select_repo_batch_action(),
            KeyCode::Backspace if form => {
                app.edit_repo_batch_form(trepo::app::state::CommitInput::Backspace)
            }
            KeyCode::Down | KeyCode::Char('j') if menu => app.move_repo_batch_selection(1),
            KeyCode::Up | KeyCode::Char('k') if menu => app.move_repo_batch_selection(-1),
            KeyCode::Down | KeyCode::Char('j') if !form => app.scroll_repo_batch(1),
            KeyCode::Up | KeyCode::Char('k') if !form => app.scroll_repo_batch(-1),
            KeyCode::PageDown if !form => app.scroll_repo_batch(10),
            KeyCode::PageUp if !form => app.scroll_repo_batch(-10),
            KeyCode::Char('c') if running => app.cancel_repo_batch(),
            KeyCode::Char('f') if !running => app.retry_failed_repo_batch(),
            KeyCode::Char(character) if form => {
                app.edit_repo_batch_form(trepo::app::state::CommitInput::Character(character))
            }
            _ => {}
        }
        return;
    }
    let graph_filter = app
        .graph
        .as_ref()
        .is_some_and(|graph| graph.filter_form.is_some());
    if app.screen == Screen::Graph && graph_filter {
        match key.code {
            KeyCode::Esc => app.cancel_graph_filter(),
            KeyCode::Enter => app.submit_graph_filter(),
            KeyCode::Backspace => app.edit_graph_filter(trepo::app::state::CommitInput::Backspace),
            KeyCode::Down | KeyCode::Tab => app.move_graph_filter_field(1),
            KeyCode::Up | KeyCode::BackTab => app.move_graph_filter_field(-1),
            KeyCode::Char(character) => {
                app.edit_graph_filter(trepo::app::state::CommitInput::Character(character))
            }
            _ => {}
        }
        return;
    }
    let graph_overlay = app
        .graph
        .as_ref()
        .is_some_and(|graph| graph.object_menu || graph.action_menu || graph.form.is_some());
    if app.screen == Screen::Graph && graph_overlay {
        let graph_form = app.graph.as_ref().is_some_and(|graph| graph.form.is_some());
        let graph_action = app.graph.as_ref().is_some_and(|graph| graph.action_menu);
        let graph_object = app.graph.as_ref().is_some_and(|graph| graph.object_menu);
        match key.code {
            KeyCode::Esc => app.cancel_graph_overlay(),
            KeyCode::Enter if graph_form => app.submit_graph_form(),
            KeyCode::Enter if graph_action => app.select_graph_action(),
            KeyCode::Enter if graph_object => app.select_graph_object(),
            KeyCode::Backspace if graph_form => {
                app.edit_graph_form(trepo::app::state::CommitInput::Backspace)
            }
            KeyCode::Char(' ') if graph_form => {
                app.edit_graph_form(trepo::app::state::CommitInput::ToggleAmend)
            }
            KeyCode::Down | KeyCode::Tab => app.move_graph_overlay_selection(1),
            KeyCode::Up | KeyCode::BackTab => app.move_graph_overlay_selection(-1),
            KeyCode::Char('j') if !graph_form => app.move_graph_overlay_selection(1),
            KeyCode::Char('k') if !graph_form => app.move_graph_overlay_selection(-1),
            KeyCode::Char(character) if graph_form => {
                app.edit_graph_form(trepo::app::state::CommitInput::Character(character))
            }
            _ => {}
        }
        return;
    }
    if app
        .repository
        .as_ref()
        .is_some_and(|state| state.pending.is_some())
    {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => app.confirm_repository_action(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.confirm_repository_action(false)
            }
            _ => {}
        }
        return;
    }
    if app
        .repository
        .as_ref()
        .is_some_and(|state| state.form.is_some())
    {
        match key.code {
            KeyCode::Esc => app.cancel_repository_overlay(),
            KeyCode::Enter => app.submit_repository_form(),
            KeyCode::Backspace => {
                app.edit_repository_form(trepo::app::state::CommitInput::Backspace)
            }
            KeyCode::Char(' ') => {
                app.edit_repository_form(trepo::app::state::CommitInput::ToggleAmend)
            }
            KeyCode::Down | KeyCode::Tab => app.move_repository_selection(1),
            KeyCode::Up | KeyCode::BackTab => app.move_repository_selection(-1),
            KeyCode::Char(character) => {
                app.edit_repository_form(trepo::app::state::CommitInput::Character(character))
            }
            _ => {}
        }
        return;
    }
    if app
        .repository
        .as_ref()
        .is_some_and(|state| state.action_menu)
    {
        match key.code {
            KeyCode::Esc | KeyCode::Char('a') => app.cancel_repository_overlay(),
            KeyCode::Enter => app.select_repository_action(),
            KeyCode::Down | KeyCode::Char('j') => app.move_repository_selection(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_repository_selection(-1),
            _ => {}
        }
        return;
    }
    if app
        .changes
        .as_ref()
        .is_some_and(|changes| changes.confirmation.is_some())
    {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => app.confirm_operation(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.confirm_operation(false),
            _ => {}
        }
        return;
    }
    if app
        .changes
        .as_ref()
        .is_some_and(|changes| changes.commit_editing)
    {
        match key.code {
            KeyCode::Esc => app.cancel_commit_editing(),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => app.submit_commit(),
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.submit_commit()
            }
            KeyCode::Enter => app.edit_commit_message(trepo::app::state::CommitInput::Newline),
            KeyCode::Backspace => {
                app.edit_commit_message(trepo::app::state::CommitInput::Backspace)
            }
            KeyCode::Delete => app.edit_commit_message(trepo::app::state::CommitInput::Delete),
            KeyCode::Left => app.edit_commit_message(trepo::app::state::CommitInput::MoveLeft),
            KeyCode::Right => app.edit_commit_message(trepo::app::state::CommitInput::MoveRight),
            KeyCode::Up => app.edit_commit_message(trepo::app::state::CommitInput::MoveUp),
            KeyCode::Down => app.edit_commit_message(trepo::app::state::CommitInput::MoveDown),
            KeyCode::Home => app.edit_commit_message(trepo::app::state::CommitInput::MoveHome),
            KeyCode::End => app.edit_commit_message(trepo::app::state::CommitInput::MoveEnd),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.edit_commit_message(trepo::app::state::CommitInput::ToggleAmend)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.edit_commit_message(trepo::app::state::CommitInput::ToggleSignoff)
            }
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.edit_commit_message(trepo::app::state::CommitInput::ToggleSigning)
            }
            KeyCode::Char(character) => {
                app.edit_commit_message(trepo::app::state::CommitInput::Character(character))
            }
            _ => {}
        }
        return;
    }
    if app.help {
        if matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
            app.help = false;
        }
        return;
    }
    if app.search_mode {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => app.search_mode = false,
            KeyCode::Backspace => {
                app.search.pop();
                app.selected = 0;
            }
            KeyCode::Char(character) => {
                app.search.push(character);
                app.selected = 0;
            }
            _ => {}
        }
        return;
    }

    match app.screen {
        Screen::Workspace => match key.code {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Esc => app.back(),
            KeyCode::Char('?') => app.help = true,
            KeyCode::Char('/') => app.search_mode = true,
            KeyCode::Char('a') => app.open_repo_batch_menu(),
            KeyCode::Char(' ') => app.toggle_project_selection(),
            KeyCode::Char('A') => app.toggle_filtered_selection(),
            KeyCode::Char('S') => app.begin_workspace_git(trepo::domain::WorkspaceGitAction::Stage),
            KeyCode::Char('Z') => app.begin_workspace_git(trepo::domain::WorkspaceGitAction::Stash),
            KeyCode::Char('D') => {
                app.begin_workspace_git(trepo::domain::WorkspaceGitAction::Discard)
            }
            KeyCode::Char('d') => app.cycle_workspace_view(),
            KeyCode::Char('r') => app.refresh(),
            KeyCode::Char('c') => app.open_changes(),
            KeyCode::Char('o') => app.open_repository(),
            KeyCode::Enter => app.open_graph(),
            KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
            KeyCode::Char('g') | KeyCode::Home => app.select_first(),
            KeyCode::Char('G') | KeyCode::End => app.select_last(),
            _ => {}
        },
        Screen::Graph => match key.code {
            KeyCode::Esc => app.back(),
            KeyCode::Char('?') => app.help = true,
            KeyCode::Char('f') => app.open_graph_filter(false),
            KeyCode::Char('/') => app.open_graph_filter(true),
            KeyCode::Char('x') => app.clear_graph_filter(),
            KeyCode::Char('r') => app.reload_graph(),
            KeyCode::Char('c') => app.open_changes(),
            KeyCode::Char('o') => app.open_repository(),
            KeyCode::Enter => app.open_graph_object_menu(),
            KeyCode::Down | KeyCode::Char('j') => app.move_graph_selection(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_graph_selection(-1),
            KeyCode::Char('g') | KeyCode::Home => app.graph_first(),
            KeyCode::Char('G') | KeyCode::End => app.graph_last(),
            _ => {}
        },
        Screen::Changes => match key.code {
            KeyCode::Esc => app.back(),
            KeyCode::Char('?') => app.help = true,
            KeyCode::Char('r') => app.reload_changes(),
            KeyCode::Tab => app.toggle_changes_mode(),
            KeyCode::Char('o') => app.open_repository(),
            KeyCode::Char(' ') => app.toggle_change_selected(),
            KeyCode::Char('A') => app.toggle_all_changes_selected(),
            KeyCode::Char('s') => app.begin_operation(trepo::domain::OperationKind::Stage),
            KeyCode::Char('u') => app.begin_operation(trepo::domain::OperationKind::Unstage),
            KeyCode::Char('z') => app.begin_operation(trepo::domain::OperationKind::Stash),
            KeyCode::Char('d') => {
                app.begin_operation(trepo::domain::OperationKind::RestoreWorktree)
            }
            KeyCode::Down | KeyCode::Char('j') => app.move_change_selection(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_change_selection(-1),
            KeyCode::Char('g') | KeyCode::Home => app.changes_first(),
            KeyCode::Char('G') | KeyCode::End => app.changes_last(),
            KeyCode::PageDown => app.scroll_preview(12),
            KeyCode::PageUp => app.scroll_preview(-12),
            KeyCode::Char('m') => app.start_commit_editing(),
            _ => {}
        },
        Screen::Repository => match key.code {
            KeyCode::Esc => app.back(),
            KeyCode::Char('?') => app.help = true,
            KeyCode::Char('r') => app.reload_repository(),
            KeyCode::Char('a') => app.toggle_repository_action_menu(),
            KeyCode::Tab => app.next_repository_tab(1),
            KeyCode::BackTab => app.next_repository_tab(-1),
            KeyCode::Down | KeyCode::Char('j') => app.move_repository_selection(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_repository_selection(-1),
            _ => {}
        },
    }
}

fn default_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get().clamp(2, 16))
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(normalize_language_args(
            args.iter().map(std::ffi::OsString::from),
        ))
    }

    #[test]
    fn language_defaults_to_english() {
        let cli = parse(&["trepo"]).unwrap();
        assert!(!cli.zh);
        assert!(!cli.en);
    }

    #[test]
    fn accepts_compatibility_and_long_language_flags() {
        assert!(parse(&["trepo", "-zh"]).unwrap().zh);
        assert!(parse(&["trepo", "--zh"]).unwrap().zh);
        assert!(parse(&["trepo", "-en"]).unwrap().en);
        assert!(parse(&["trepo", "--en"]).unwrap().en);
    }

    #[test]
    fn rejects_conflicting_language_flags() {
        assert!(parse(&["trepo", "-zh", "--en"]).is_err());
    }
}
