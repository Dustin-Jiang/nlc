use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
};

use crate::app::{App, FileAction, Focus, Mode};

/// [[docs/003-ui-design.md#颜色主题]]
const TITLE_BG: Color = Color::Blue;
const MAT_A_FG: Color = Color::Cyan;
const MAT_B_FG: Color = Color::Yellow;
const RESULT_FG: Color = Color::Green;
const SELECTED_BG: Color = Color::White;
const SELECTED_FG: Color = Color::Black;
const ERROR_FG: Color = Color::Red;
const HELP_FG: Color = Color::Gray;
const INPUT_FG: Color = Color::Gray;

/// [[docs/003-ui-design.md#渲染流程]]
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    if area.width < 80 || area.height < 24 {
        render_small_terminal(frame, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    render_title(frame, chunks[0], app);
    render_matrix_area(frame, chunks[1], app);
    render_input_line(frame, chunks[2], app);
    render_operation_bar(frame, chunks[3], app);
    render_status_bar(frame, chunks[4], app);

    if app.show_help {
        render_help_overlay(frame, area);
    }

    if app.pending_confirmation.is_some() {
        render_dialog(frame, area, app);
    }
}

fn render_small_terminal(frame: &mut Frame, area: Rect) {
    let msg = Paragraph::new("终端太小，请放大至 80×24 以上")
        .style(Style::new().fg(ERROR_FG).add_modifier(Modifier::BOLD))
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(msg, area);
}

/// [[docs/003-ui-design.md#布局结构]]
fn render_title(frame: &mut Frame, area: Rect, app: &App) {
    let title = Line::from(vec![
        Span::styled(
            " 矩阵计算器 v1.0 ",
            Style::new()
                .fg(Color::White)
                .bg(TITLE_BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{} {}", app.focus_label(), mode_label(&app.mode)),
            Style::new().fg(Color::White).bg(TITLE_BG),
        ),
    ]);
    frame.render_widget(title, area);
}

fn mode_label(mode: &Mode) -> &'static str {
    match mode {
        Mode::Browsing => "浏览",
        Mode::EditingCell => "编辑",
        Mode::EnteringData => "输入",
    }
}

/// [[docs/003-ui-design.md#布局结构]]
fn render_matrix_area(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(area);

    render_matrix_widget(frame, chunks[0], "矩阵 A", &app.mat_a, MAT_A_FG, app.focus == Focus::MatrixA, app);

    if let Some(ref mat_b) = app.mat_b {
        render_matrix_widget(frame, chunks[1], "矩阵 B", mat_b, MAT_B_FG, app.focus == Focus::MatrixB, app);
    } else {
        render_empty_slot(frame, chunks[1], "矩阵 B", MAT_B_FG, app.focus == Focus::MatrixB);
    }

    if let Some(ref result) = app.result {
        render_matrix_widget(frame, chunks[2], "结果", result, RESULT_FG, app.focus == Focus::Result, app);
    } else {
        render_empty_slot(frame, chunks[2], "结果", RESULT_FG, app.focus == Focus::Result);
    }
}

/// [[docs/003-ui-design.md#matrixwidget]]
fn render_matrix_widget(frame: &mut Frame, area: Rect, label: &str, mat: &crate::matrix::Matrix, color: Color, focused: bool, app: &App) {
    let border_style = if focused {
        Style::new()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else if app.focus == Focus::Result {
        Style::new().fg(Color::DarkGray)
    } else {
        Style::new().fg(color)
    };

    let block = Block::default()
        .title(format!(" {label} "))
        .borders(Borders::ALL)
        .border_style(border_style);

    if mat.is_empty() {
        if app.mode == Mode::EnteringData && focused {
            let text = if app.input_buf.is_empty() {
                "输入矩阵数据...".to_string()
            } else {
                app.input_buf.clone()
            };
            let para = Paragraph::new(text.as_str())
                .style(Style::new().fg(INPUT_FG))
                .block(block);
            frame.render_widget(para, area);
        } else {
            let para = Paragraph::new("[空]")
                .style(Style::new().fg(color))
                .block(block);
            frame.render_widget(para, area);
        }
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(&block, area);

    // 列标签：C1, C2, ...
    let mut header_cells = vec![Cell::from("     ")];
    for c in 0..mat.cols {
        header_cells.push(Cell::from(format!("  C{}    ", c + 1)));
    }
    let mut rows = vec![Row::new(header_cells)];
    for r in 0..mat.rows {
        let mut cells = vec![Cell::from(format!(" R{} ", r + 1)).style(Style::new().fg(color))];
        for c in 0..mat.cols {
            let val_str = format!("{:8.4}", mat.data[r][c]);
            let is_selected = focused && app.mode != Mode::EnteringData && r == app.selected.0 && c == app.selected.1;

            let cell_str = if is_selected && app.mode == Mode::EditingCell {
                app.input_buf.clone()
            } else {
                val_str
            };
            let cell = if is_selected {
                Cell::from(cell_str)
                    .style(Style::new().bg(SELECTED_BG).fg(SELECTED_FG))
            } else {
                Cell::from(cell_str)
            };
            cells.push(cell);
        }
        rows.push(Row::new(cells));
    }

    let mut widths: Vec<Constraint> = vec![Constraint::Length(5)];
    widths.extend((0..mat.cols).map(|_| Constraint::Length(10)));

    let table = Table::new(rows, widths)
        .column_spacing(0)
        .block(Block::default());
    frame.render_widget(table, inner);
}

fn render_empty_slot(frame: &mut Frame, area: Rect, label: &str, color: Color, focused: bool) {
    let border_style = if focused {
        Style::new()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(color)
    };

    let block = Block::default()
        .title(format!(" {label} "))
        .borders(Borders::ALL)
        .border_style(border_style);

    let para = Paragraph::new("[空]")
        .style(Style::new().fg(color))
        .block(block);
    frame.render_widget(para, area);
}

/// [[docs/003-ui-design.md#布局结构]]
fn render_input_line(frame: &mut Frame, area: Rect, app: &App) {
    let text: String = match (&app.file_action, &app.mode, app.input_buf.is_empty()) {
        (Some(FileAction::Save), _, _) => format!("> 输入文件名: {}", app.input_buf),
        (Some(FileAction::Load), _, _) => format!("> 输入文件名: {}", app.input_buf),
        (None, Mode::Browsing, true) => "输入矩阵元素值".to_string(),
        (None, Mode::Browsing, false) => format!("> {}", app.input_buf),
        (None, Mode::EditingCell, _) => format!("> {}", app.input_buf),
        (None, Mode::EnteringData, _) => format!("> {}", app.input_buf),
    };
    let para = Paragraph::new(text.as_str())
        .style(Style::new().fg(INPUT_FG));
    frame.render_widget(para, area);
}

/// [[docs/003-ui-design.md#operationbar]]
fn render_operation_bar(frame: &mut Frame, area: Rect, app: &App) {
    let ops = match app.mode {
        Mode::Browsing => {
            "[+] [-] [×] [÷] [T] [I] [D] [E] [H]"
        }
        Mode::EditingCell | Mode::EnteringData => {
            "[Esc] [Enter]"
        }
    };
    let bar = Line::from(vec![
        Span::styled(ops, Style::new().fg(Color::White)),
    ]);
    frame.render_widget(bar, area);
}

/// [[docs/003-ui-design.md#statusbar]]
fn render_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let (text, style) = if let Some(ref error) = app.error {
        (error.clone(), Style::new().fg(ERROR_FG).add_modifier(Modifier::BOLD))
    } else if app.show_history_once && !app.history.is_empty() {
        let recent: Vec<&str> = app.history.iter().rev().take(3).map(|s| s.as_str()).collect();
        (recent.join(" · "), Style::new().fg(Color::Cyan))
    } else {
        let hint = match app.mode {
            Mode::Browsing => "Tab切换 ↑↓←→移动 Enter编辑 +-*/运算",
            Mode::EditingCell => "输入数值 · Enter确认 Esc取消",
            Mode::EnteringData => "行用;分隔 · Enter完成 Esc取消",
        };
        (hint.to_string(), Style::new().fg(HELP_FG))
    };

    let para = Paragraph::new(text.as_str()).style(style);
    frame.render_widget(para, area);
}

/// [[docs/004-interaction-design.md#5-帮助系统]]
fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let help_text = "\
┌──────────── 帮助 ────────────┐
│                              │
│  导航                         │
│  Tab       切换焦点矩阵        │
│  ↑↓←→      移动选中单元格      │
│                              │
│  编辑                         │
│  Enter     编辑/输入矩阵       │
│  Esc       取消/返回           │
│  c         清空矩阵            │
│                              │
│  运算                         │
│  + - * /   加减乘除           │
│  T         转置               │
│  I         求逆               │
│  D         行列式             │
│  E         特征值             │
│                              │
│  文件                         │
│  S         保存到文件          │
│  L         从文件加载          │
│                              │
│  历史                         │
│  U         查看操作历史        │
│                              │
│  系统                         │
│  H         切换此帮助          │
│  Q/Esc     退出程序            │
│  Ctrl-C    全局退出            │
└──────────────────────────────┘";

    let overlay = Paragraph::new(help_text)
        .style(Style::new().fg(Color::White).bg(Color::Black));

    let help_w = 34;
    let help_h = 22;
    let x = area.x + (area.width.saturating_sub(help_w)) / 2;
    let y = area.y + (area.height.saturating_sub(help_h)) / 2;
    let overlay_area = Rect { x, y, width: help_w, height: help_h };

    frame.render_widget(Clear, overlay_area);
    frame.render_widget(overlay, overlay_area);
}

/// [[docs/003-ui-design.md#dialog]]
/// 使用组件库 Block + Paragraph 构建确认对话框覆盖层。
fn render_dialog(frame: &mut Frame, area: Rect, app: &App) {
    let text = app.pending_confirmation.as_deref().unwrap_or("");
    let dialog = Paragraph::new(text)
        .style(Style::new().fg(Color::White).bg(Color::Black))
        .alignment(ratatui::layout::Alignment::Center)
        .block(
            Block::default()
                .title(" 确认 ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Yellow)),
        );

    let w = 34;
    let h = 5;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let dialog_area = Rect { x, y, width: w, height: h };

    frame.render_widget(Clear, dialog_area);
    frame.render_widget(dialog, dialog_area);
}
