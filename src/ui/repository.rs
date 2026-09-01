use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::repository::{action_preview_with_language, choices, FormField, RepositoryTab};
use crate::app::state::{App, RepositoryState};
use crate::domain::{RepositoryAction, RiskLevel};

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let Some(state) = app.repository.as_ref() else {
        frame.render_widget(
            Paragraph::new(app.language.text("No repository selected", "未选择仓库")),
            area,
        );
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
    render_header(frame, app, state, vertical[0]);
    render_tabs(frame, app, state, vertical[1]);
    if area.width >= 100 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(vertical[2]);
        render_items(frame, app, state, body[0]);
        render_detail(frame, app, state, body[1]);
    } else {
        render_items(frame, app, state, vertical[2]);
    }
    render_footer(frame, app, state, vertical[3]);
    if state.action_menu {
        render_action_menu(frame, app, state);
    }
    if state.form.is_some() {
        render_form(frame, app, state);
    }
    if state.pending.is_some() {
        render_confirmation(frame, app, state);
    }
}

fn render_header(frame: &mut Frame, app: &App, state: &RepositoryState, area: Rect) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " trepo ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {}  /  {}",
                state.project.name,
                app.language.text("Repository", "仓库")
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_tabs(frame: &mut Frame, app: &App, state: &RepositoryState, area: Rect) {
    let spans = RepositoryTab::ALL.into_iter().flat_map(|tab| {
        let style = if tab == state.tab {
            super::selection_style()
        } else {
            Style::default().fg(Color::Gray)
        };
        [
            Span::styled(format!(" {} ", repository_tab_label(app, tab)), style),
            Span::raw(" "),
        ]
    });
    frame.render_widget(Paragraph::new(Line::from_iter(spans)), area);
}

fn render_items(frame: &mut Frame, app: &App, state: &RepositoryState, area: Rect) {
    let block = Block::default()
        .title(format!(" {} ", repository_tab_label(app, state.tab)))
        .borders(Borders::ALL);
    if state.loading {
        frame.render_widget(
            Paragraph::new(
                app.language
                    .text("Loading repository state...", "正在加载仓库状态..."),
            )
            .block(block),
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
            Paragraph::new(
                app.language
                    .text("No repository state available", "没有可用的仓库状态"),
            )
            .block(block),
            area,
        );
        return;
    };
    let values = match state.tab {
        RepositoryTab::Status => {
            let mut values = vec![format!(
                "{}: {}",
                app.language.label("Operation"),
                snapshot
                    .operation
                    .map_or(app.language.label("none"), |operation| {
                        app.language.label(operation.label())
                    })
            )];
            values.extend(
                snapshot
                    .conflicts
                    .iter()
                    .map(|path| format!("{}  {}", app.language.label("Conflict"), path.display())),
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
        vec![ListItem::new(app.language.text("No entries", "没有条目"))]
    } else {
        values
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                ListItem::new(super::text::truncate(
                    &value,
                    area.width.saturating_sub(2) as usize,
                ))
                .style(if index == state.selected {
                    super::selection_style()
                } else {
                    Style::default()
                })
            })
            .collect()
    };
    frame.render_widget(List::new(items).block(block), area);
}

fn render_detail(frame: &mut Frame, app: &App, state: &RepositoryState, area: Rect) {
    let text = state.detail.as_deref().unwrap_or_else(|| {
        state.message.as_ref().map_or(
            app.language.text(
                "Select an entry or open Actions.",
                "选择一个条目或打开操作菜单。",
            ),
            |(_, message)| message.as_str(),
        )
    });
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .title(app.language.text(" Detail ", " 详情 "))
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame, app: &App, state: &RepositoryState, area: Rect) {
    let status = if state.action_running {
        Span::styled(
            app.language
                .text("Running Git operation...", "正在执行 Git 操作..."),
            Style::default().fg(Color::Yellow),
        )
    } else if let Some((error, message)) = &state.message {
        Span::styled(
            message.clone(),
            Style::default().fg(if *error { Color::Red } else { Color::Green }),
        )
    } else {
        Span::raw(app.language.text(
            "Tab/Shift-Tab Views  a Actions  j/k Move  r Refresh  Esc Back",
            "Tab/Shift-Tab 视图  a 操作  j/k 移动  r 刷新  Esc 返回",
        ))
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(status),
            Line::raw(app.language.text(
                "Enter selects menu/form item  Space toggles option",
                "Enter 选择菜单/表单项  Space 切换选项",
            )),
        ]),
        area,
    );
}

fn render_action_menu(frame: &mut Frame, app: &App, state: &RepositoryState) {
    let area = centered_rect(58, 70, frame.area());
    frame.render_widget(Clear, area);
    let items = choices(state.tab)
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            ListItem::new(app.language.action(choice.label())).style(
                if index == state.action_selected {
                    super::selection_style()
                } else {
                    Style::default()
                },
            )
        });
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(app.language.text(" Actions ", " 操作 "))
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_form(frame: &mut Frame, app: &App, state: &RepositoryState) {
    let Some(form) = state.form.as_ref() else {
        return;
    };
    let area = centered_rect(72, 68, frame.area());
    frame.render_widget(Clear, area);
    let lines = form.fields.iter().enumerate().map(|(index, field)| {
        let prefix = if index == form.selected { ">" } else { " " };
        let style = if index == form.selected {
            super::selection_style()
        } else {
            Style::default()
        };
        let hint = if matches!(field, FormField::Toggle { .. }) {
            " [Space]"
        } else {
            ""
        };
        let value = localized_field_value(app, field);
        Line::styled(
            format!(
                "{prefix} {}: {value}{hint}",
                app.language.label(field.label())
            ),
            style,
        )
    });
    let mut lines = lines.collect::<Vec<_>>();
    lines.push(Line::raw(""));
    lines.push(Line::raw(
        app.language
            .text("Enter execute   Esc cancel", "Enter 执行   Esc 取消"),
    ));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(format!(" {} ", app.language.action(form.choice.label())))
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_confirmation(frame: &mut Frame, app: &App, state: &RepositoryState) {
    let Some(action) = state.pending.as_ref() else {
        return;
    };
    let area = centered_rect(68, 60, frame.area());
    frame.render_widget(Clear, area);
    let risk = match action {
        RepositoryAction::Push {
            force_with_lease: true,
            ..
        } => app.language.text(
            "This action may rewrite remote branch history and uses force-with-lease.",
            "此操作可能重写远程分支历史，并使用租约强制推送。",
        ),
        _ => match action.risk() {
            RiskLevel::RemoteWrite => app.language.text(
                "This action writes to a remote repository.",
                "此操作会写入远程仓库。",
            ),
            RiskLevel::Destructive => app.language.text(
                "This action can discard local or reference state.",
                "此操作可能丢弃本地改动或引用状态。",
            ),
            _ => app
                .language
                .text("Confirm this repository operation.", "请确认此仓库操作。"),
        },
    };
    let mut lines = vec![
        Line::styled(
            risk,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::raw(format!(
            "{}: {}",
            app.language.text("Action", "操作"),
            app.language.action(action.label())
        )),
    ];
    if let Some(snapshot) = state.snapshot.as_ref() {
        lines.extend(
            action_preview_with_language(app.language, action, snapshot)
                .into_iter()
                .map(Line::raw),
        );
    }
    lines.extend([
        Line::raw(""),
        Line::raw(app.language.text(
            "Press y to continue or n/Esc to cancel.",
            "按 y 继续，按 n/Esc 取消。",
        )),
    ]);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(app.language.text(" Confirm operation ", " 确认操作 "))
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn localized_field_value(app: &App, field: &FormField) -> String {
    match field {
        FormField::Toggle { value, .. } => app
            .language
            .label(if *value { "on" } else { "off" })
            .to_owned(),
        FormField::Text { value, .. } => value.clone(),
    }
}

fn repository_tab_label(app: &App, tab: RepositoryTab) -> &'static str {
    match tab {
        RepositoryTab::Status => app.language.text("Status", "状态"),
        RepositoryTab::Stashes => app.language.text("Stashes", "储藏"),
        RepositoryTab::Refs => app.language.text("Branches & Tags", "分支与标签"),
        RepositoryTab::Remotes => app.language.text("Remotes", "远程"),
    }
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
