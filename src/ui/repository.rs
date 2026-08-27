use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::repository::{action_preview, choices, FormField, RepositoryTab};
use crate::app::state::{App, RepositoryState};
use crate::domain::{RepositoryAction, RiskLevel};

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let Some(state) = app.repository.as_ref() else {
        frame.render_widget(Paragraph::new("No repository selected"), area);
        return;
    };
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(area);
    render_header(frame, state, vertical[0]);
    render_tabs(frame, state, vertical[1]);
    if area.width >= 100 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(vertical[2]);
        render_items(frame, state, body[0]);
        render_detail(frame, state, body[1]);
    } else {
        render_items(frame, state, vertical[2]);
    }
    render_footer(frame, state, vertical[3]);
    if state.action_menu {
        render_action_menu(frame, state);
    }
    if state.form.is_some() {
        render_form(frame, state);
    }
    if state.pending.is_some() {
        render_confirmation(frame, state);
    }
}

fn render_header(frame: &mut Frame, state: &RepositoryState, area: Rect) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " repo-tui ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {}  /  Repository", state.project.name)),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_tabs(frame: &mut Frame, state: &RepositoryState, area: Rect) {
    let spans = RepositoryTab::ALL.into_iter().flat_map(|tab| {
        let style = if tab == state.tab {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        [
            Span::styled(format!(" {} ", tab.label()), style),
            Span::raw(" "),
        ]
    });
    frame.render_widget(Paragraph::new(Line::from_iter(spans)), area);
}

fn render_items(frame: &mut Frame, state: &RepositoryState, area: Rect) {
    let block = Block::default()
        .title(format!(" {} ", state.tab.label()))
        .borders(Borders::ALL);
    if state.loading {
        frame.render_widget(
            Paragraph::new("Loading repository state...").block(block),
            area,
        );
        return;
    }
    if let Some(error) = &state.error {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::default().fg(Color::Red))
                .block(block),
            area,
        );
        return;
    }
    let Some(snapshot) = state.snapshot.as_ref() else {
        frame.render_widget(
            Paragraph::new("No repository state available").block(block),
            area,
        );
        return;
    };
    let values = match state.tab {
        RepositoryTab::Status => {
            let mut values = vec![format!(
                "Operation: {}",
                snapshot
                    .operation
                    .map_or("none", |operation| operation.label())
            )];
            values.extend(
                snapshot
                    .conflicts
                    .iter()
                    .map(|path| format!("Conflict  {}", path.display())),
            );
            values
        }
        RepositoryTab::Stashes => snapshot
            .stashes
            .iter()
            .map(|stash| {
                format!(
                    "{}  {}  {}",
                    stash.selector,
                    short(&stash.oid),
                    stash.subject
                )
            })
            .collect(),
        RepositoryTab::Refs => snapshot
            .branches
            .iter()
            .map(|branch| {
                format!(
                    "{} {}  {}  {}  +{} -{}",
                    if branch.current { "*" } else { " " },
                    branch.name,
                    short(&branch.oid),
                    branch.upstream.as_deref().unwrap_or("-"),
                    branch.ahead,
                    branch.behind
                )
            })
            .chain(
                snapshot
                    .tags
                    .iter()
                    .map(|tag| format!("T {}  {}", tag.name, short(&tag.target))),
            )
            .collect(),
        RepositoryTab::Remotes => snapshot
            .remotes
            .iter()
            .map(|remote| format!("{}  {}", remote.name, remote.fetch_url))
            .collect(),
    };
    let items = if values.is_empty() {
        vec![ListItem::new("No entries")]
    } else {
        values
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                ListItem::new(value).style(if index == state.selected {
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                })
            })
            .collect()
    };
    frame.render_widget(List::new(items).block(block), area);
}

fn render_detail(frame: &mut Frame, state: &RepositoryState, area: Rect) {
    let text = state.detail.as_deref().unwrap_or_else(|| {
        state
            .message
            .as_ref()
            .map_or("Select an entry or open Actions.", |(_, message)| {
                message.as_str()
            })
    });
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().title(" Detail ").borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame, state: &RepositoryState, area: Rect) {
    let status = if state.action_running {
        Span::styled(
            "Running Git operation...",
            Style::default().fg(Color::Yellow),
        )
    } else if let Some((error, message)) = &state.message {
        Span::styled(
            message.clone(),
            Style::default().fg(if *error { Color::Red } else { Color::Green }),
        )
    } else {
        Span::raw("Tab/Shift-Tab Views  a Actions  j/k Move  r Refresh  Esc Back")
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(status),
            Line::raw("Enter selects menu/form item  Space toggles option"),
        ]),
        area,
    );
}

fn render_action_menu(frame: &mut Frame, state: &RepositoryState) {
    let area = centered_rect(58, 70, frame.area());
    frame.render_widget(Clear, area);
    let items = choices(state.tab)
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            ListItem::new(choice.label()).style(if index == state.action_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            })
        });
    frame.render_widget(
        List::new(items).block(Block::default().title(" Actions ").borders(Borders::ALL)),
        area,
    );
}

fn render_form(frame: &mut Frame, state: &RepositoryState) {
    let Some(form) = state.form.as_ref() else {
        return;
    };
    let area = centered_rect(72, 68, frame.area());
    frame.render_widget(Clear, area);
    let lines = form.fields.iter().enumerate().map(|(index, field)| {
        let prefix = if index == form.selected { ">" } else { " " };
        let style = if index == form.selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let hint = if matches!(field, FormField::Toggle { .. }) {
            " [Space]"
        } else {
            ""
        };
        Line::styled(
            format!(
                "{prefix} {}: {}{hint}",
                field.label(),
                field.display_value()
            ),
            style,
        )
    });
    let mut lines = lines.collect::<Vec<_>>();
    lines.push(Line::raw(""));
    lines.push(Line::raw("Enter execute   Esc cancel"));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(format!(" {} ", form.choice.label()))
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_confirmation(frame: &mut Frame, state: &RepositoryState) {
    let Some(action) = state.pending.as_ref() else {
        return;
    };
    let area = centered_rect(68, 60, frame.area());
    frame.render_widget(Clear, area);
    let risk = match action {
        RepositoryAction::Push {
            force_with_lease: true,
            ..
        } => "This action may rewrite remote branch history and uses force-with-lease.",
        _ => match action.risk() {
            RiskLevel::RemoteWrite => "This action writes to a remote repository.",
            RiskLevel::Destructive => "This action can discard local or reference state.",
            _ => "Confirm this repository operation.",
        },
    };
    let mut lines = vec![
        Line::styled(
            risk,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::raw(format!("Action: {}", action.label())),
    ];
    if let Some(snapshot) = state.snapshot.as_ref() {
        lines.extend(action_preview(action, snapshot).into_iter().map(Line::raw));
    }
    lines.extend([
        Line::raw(""),
        Line::raw("Press y to continue or n/Esc to cancel."),
    ]);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Confirm operation ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let width = area.width.saturating_mul(percent_x) / 100;
    let height = area.height.saturating_mul(percent_y) / 100;
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.max(1),
        height.max(1),
    )
}

fn short(value: &str) -> String {
    value.chars().take(10).collect()
}
