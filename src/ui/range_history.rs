use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::state::{App, RangeHistoryState};

/// Renders the workspace range-query overlay: the filter form, the merged
/// cross-repository commit timeline, or the status/error message.
pub(super) fn render(frame: &mut Frame, app: &App, state: &RangeHistoryState) {
    if let Some(form) = state.form.as_ref() {
        render_form(frame, app, form);
        return;
    }
    render_results(frame, app, state);
}

fn render_form(frame: &mut Frame, app: &App, form: &crate::app::state::RangeHistoryForm) {
    let area = centered_rect(66, 40, frame.area());
    frame.render_widget(Clear, area);
    let mut lines = Vec::new();
    for (index, (label, value)) in form.fields().iter().enumerate() {
        let marker = if index == form.selected { "> " } else { "  " };
        let style = if index == form.selected {
            super::selection_style()
        } else {
            Style::default()
        };
        lines.push(Line::styled(
            format!("{marker}{}: {value}", app.language.label(label)),
            style,
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        app.language.text(
            "Enter run   Tab field   Esc cancel",
            "Enter 执行   Tab 字段   Esc 取消",
        ),
        Color::DarkGray,
    ));
    lines.push(Line::styled(
        app.language.text(
            "Dates use YYYY-MM-DD; empty means unbounded",
            "日期格式 YYYY-MM-DD；留空表示不限制",
        ),
        Color::DarkGray,
    ));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(app.language.text(" Range history ", " 时间范围检索 "))
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_results(frame: &mut Frame, app: &App, state: &RangeHistoryState) {
    let area = centered_rect(92, 80, frame.area());
    frame.render_widget(Clear, area);

    let rows = display_rows(app);
    let error_count = state.projects.iter().filter(|p| p.error.is_some()).count();
    let title = format!(
        " {} ({} {}{}) ",
        app.language.text("Range history", "时间范围检索"),
        rows.len(),
        app.language.text("commits", "提交"),
        if error_count > 0 {
            format!(
                ", {} {}",
                error_count,
                app.language.text("repositories failed", "个仓库失败")
            )
        } else {
            String::new()
        }
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let footer = if state.loading {
        app.language
            .text("Loading...   Esc close", "正在加载...   Esc 关闭")
    } else {
        app.language.text(
            "Esc close   r rerun   f filter",
            "Esc 关闭   r 重跑   f 过滤",
        )
    };

    if rows.is_empty() {
        let message = if state.loading {
            app.language.text("Loading...", "正在加载...")
        } else if let Some((_, text)) = state.message.as_ref() {
            text.as_str()
        } else {
            app.language.text("No commits in range", "范围内没有提交")
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::raw(message.to_owned()),
                Line::raw(""),
                Line::styled(footer, Color::DarkGray),
            ])
            .block(block)
            .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let header = Row::new([
        Cell::from(app.language.label("Project")),
        Cell::from(app.language.label("Commit")),
        Cell::from(app.language.label("Date")),
        Cell::from(app.language.label("Author")),
        Cell::from(app.language.label("Subject")),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let selected = state.selected.min(rows.len() - 1);
    let body = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let style = if index == selected {
                super::selection_style()
            } else {
                Style::default()
            };
            Row::new([
                Cell::from(row.0.clone()),
                Cell::from(row.1.clone()),
                Cell::from(row.2.clone()),
                Cell::from(row.3.clone()),
                Cell::from(row.4.clone()),
            ])
            .style(style)
        })
        .collect::<Vec<_>>();

    let widths = [
        Constraint::Length(18),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Min(10),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .block(block)
        .row_highlight_style(super::selection_style());
    frame.render_widget(table, area);

    if area.height > 2 {
        let footer_area = Rect::new(area.x + 1, area.y + area.height - 1, area.width - 2, 1);
        frame.render_widget(
            Paragraph::new(Line::styled(footer, Color::DarkGray)),
            footer_area,
        );
    }
}

/// Flattens the per-project results into display rows, newest first, using the
/// same ordering the state exposes so selection stays consistent.
fn display_rows(app: &App) -> Vec<(String, String, String, String, String)> {
    app.range_history_rows()
        .into_iter()
        .map(|(project, oid, commit)| {
            (
                super::text::truncate(&project, 18),
                oid,
                super::graph::calendar_date(commit.timestamp),
                super::text::truncate(&commit.author.replace(['\n', '\r'], " "), 14),
                super::text::truncate(&commit.subject.replace(['\n', '\r'], " "), 200),
            )
        })
        .collect()
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
