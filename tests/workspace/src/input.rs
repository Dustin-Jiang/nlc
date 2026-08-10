use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, FileAction, Focus, Mode};
use crate::matrix::Matrix;

/// [[docs/004-interaction-design.md#2-输入事件流]]
pub fn handle_key(app: &mut App, key: KeyEvent) {
    if app.pending_confirmation.is_some() {
        handle_confirmation(app, key);
        return;
    }

    if app.show_help {
        match key.code {
            KeyCode::Char('h' | 'H') => app.show_help = false,
            KeyCode::Char('q' | 'Q') => {
                app.pending_confirmation = Some("确认退出? Y/N".into());
            }
            _ => {}
        }
        return;
    }

    // Ctrl-C 全局退出
    if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
        app.pending_confirmation = Some("确认退出? Y/N".into());
        return;
    }

    if app.file_action.is_some() {
        handle_file_naming(app, key);
        return;
    }

    match app.mode {
        Mode::Browsing => handle_browsing(app, key),
        Mode::EditingCell => handle_editing_cell(app, key),
        Mode::EnteringData => handle_entering_data(app, key),
    }
}

fn handle_confirmation(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
            let action = app.pending_confirmation.take();
            if let Some(ref action_str) = action {
                if action_str.contains("退出") {
                    app.quit = true;
                } else if action_str.contains("清空") {
                    app.clear_focused();
                    app.mode = Mode::EnteringData;
                    app.input_buf.clear();
                    app.error = None;
                } else if action_str.contains("覆盖") {
                    let label = app.focus_label().trim_matches(&['[', ']'][..]).to_lowercase();
                    if action_str.contains("文件将覆盖") {
                        // Load overwrite: proceed to file naming
                        app.input_buf = format!("matrix_{label}.txt");
                        app.file_action = Some(FileAction::Load);
                        app.error = None;
                    } else if action_str.contains("文件已存在") {
                        // Save overwrite: read filename from input_buf, execute save
                        let filename = app.input_buf.clone();
                        app.input_buf.clear();
                        do_save(app, &filename);
                    }
                }
            }
        }
        KeyCode::Char('n' | 'N') | KeyCode::Esc => {
            app.pending_confirmation = None;
            app.error = None;
        }
        _ => {}
    }
}

/// [[docs/004-interaction-design.md#22-按键分发表-browsing-模式]]
fn handle_browsing(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Tab => app.shift_focus(),

        KeyCode::Up => {
            if app.selected.0 > 0 {
                app.selected.0 -= 1;
            }
        }
        KeyCode::Down => {
            if let Some(mat) = app.focused_matrix()
                && app.selected.0 + 1 < mat.rows
            {
                app.selected.0 += 1;
            }
        }
        KeyCode::Left => {
            if app.selected.1 > 0 {
                app.selected.1 -= 1;
            }
        }
        KeyCode::Right => {
            if let Some(mat) = app.focused_matrix()
                && app.selected.1 + 1 < mat.cols
            {
                app.selected.1 += 1;
            }
        }

        KeyCode::Enter => start_editing(app),

        KeyCode::Char('+') => binary_op(app, "Add"),
        KeyCode::Char('-') => binary_op(app, "Sub"),
        KeyCode::Char('*') => binary_op(app, "Mul"),
        KeyCode::Char('/') => binary_op(app, "Div"),

        KeyCode::Char('t' | 'T') => unary_op(app, "Transpose"),
        KeyCode::Char('i' | 'I') => unary_op(app, "Inverse"),
        KeyCode::Char('d' | 'D') => unary_op(app, "Det"),
        KeyCode::Char('e' | 'E') => unary_op(app, "Eigen"),

        KeyCode::Char('c' | 'C') => {
            if app.is_focused_empty() {
                app.clear_focused();
                app.mode = Mode::EnteringData;
                app.input_buf.clear();
                app.error = None;
            } else {
                app.pending_confirmation = Some("确认清空? Y/N".into());
            }
        }

        KeyCode::Char('s' | 'S') => {
            if app.is_focused_empty() {
                app.error = Some("矩阵为空，无法保存".into());
            } else {
                let label = app.focus_label().trim_matches(&['[', ']'][..]).to_lowercase();
                app.input_buf = format!("matrix_{label}.txt");
                app.file_action = Some(FileAction::Save);
                app.error = None;
            }
        }
        KeyCode::Char('l' | 'L') => {
            if !app.is_focused_empty() && app.focus != Focus::Result {
                app.pending_confirmation = Some("文件将覆盖当前矩阵, 确认? Y/N".into());
            } else {
                let label = app.focus_label().trim_matches(&['[', ']'][..]).to_lowercase();
                app.input_buf = format!("matrix_{label}.txt");
                app.file_action = Some(FileAction::Load);
                app.error = None;
            }
        }

        KeyCode::Char('h' | 'H') => app.show_help = !app.show_help,

        KeyCode::Char('u' | 'U') => {
            if app.history.is_empty() {
                app.error = Some("暂无操作历史".into());
            } else {
                app.show_history_once = true;
            }
        }

        KeyCode::Esc | KeyCode::Char('q' | 'Q') => {
            app.pending_confirmation = Some("确认退出? Y/N".into());
        }

        _ => {
            app.error = Some("无效按键".into());
        }
    }
}

/// [[docs/004-interaction-design.md#131-模式切换规则]]
fn start_editing(app: &mut App) {
    if app.focus == Focus::Result {
        app.error = Some("结果矩阵只读，无法编辑".into());
        return;
    }
    if app.is_focused_empty() {
        app.mode = Mode::EnteringData;
        app.input_buf.clear();
        app.error = None;
        return;
    }
    let mat = match app.focused_matrix() {
        Some(m) => m,
        None => return,
    };
    if app.selected.0 >= mat.rows || app.selected.1 >= mat.cols {
        app.error = Some("索引越界".into());
        return;
    }
    app.input_buf = format!("{}", mat.data[app.selected.0][app.selected.1]);
    app.mode = Mode::EditingCell;
    app.error = None;
}

/// [[docs/004-interaction-design.md#132-按键分发表-editingcell-模式]]
fn handle_editing_cell(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char(c) if c.is_ascii_digit() || c == '.' || c == '-' => {
            if c == '.' && app.input_buf.contains('.') {
                return;
            }
            app.input_buf.push(c);
        }
        KeyCode::Backspace => {
            app.input_buf.pop();
        }
        KeyCode::Enter => commit_edit(app),
        KeyCode::Esc => {
            app.mode = Mode::Browsing;
            app.error = None;
        }
        _ => {}
    }
}

fn commit_edit(app: &mut App) {
    let val: f64 = match app.input_buf.parse() {
        Ok(v) => v,
        Err(_) => {
            app.error = Some("无效数字".into());
            return;
        }
    };
    let (row, col) = app.selected;
    let label = app.focus_label().to_string();
    if let Some(mat) = app.focused_matrix_mut() {
        let old = mat.data[row][col];
        mat.data[row][col] = val;
        app.history.push(format!(
            "{label}[{},{}]: {:.4} → {:.4}",
            row + 1,
            col + 1,
            old,
            val
        ));
    }
    app.mode = Mode::Browsing;
    app.error = None;
}

/// [[docs/004-interaction-design.md#132-按键分表-enteringdata-模式]]
fn handle_entering_data(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char(c)
            if c.is_ascii_digit() || c == '.' || c == '-' || c == ';' || c == ' ' =>
        {
            app.input_buf.push(c);
        }
        KeyCode::Backspace => {
            app.input_buf.pop();
        }
        KeyCode::Enter => commit_matrix(app),
        KeyCode::Esc => {
            app.mode = Mode::Browsing;
            app.input_buf.clear();
            app.error = None;
        }
        _ => {}
    }
}

fn commit_matrix(app: &mut App) {
    let mat: Result<Matrix, String> = app.input_buf.parse();
    match mat {
        Ok(m) => {
            let label = app.focus_label().to_string();
            let dim = format!("{}×{}", m.rows, m.cols);
            match app.focus {
                Focus::MatrixA => app.mat_a = m,
                Focus::MatrixB => app.mat_b = Some(m),
                Focus::Result => app.result = Some(m),
            }
            app.mode = Mode::Browsing;
            app.error = None;
            app.history.push(format!("{label} {dim} 加载成功"));
        }
        Err(e) => {
            app.error = Some(format!("解析失败: {e}"));
        }
    }
}

/// [[docs/004-interaction-design.md#43-运算触发交互]]
fn binary_op(app: &mut App, op: &str) {
    let a = &app.mat_a;
    let b = match app.mat_b {
        Some(ref m) => m,
        None => {
            app.error = Some("矩阵B为空，无法运算".into());
            return;
        }
    };

    if a.is_empty() {
        app.error = Some("矩阵A为空，无法运算".into());
        return;
    }

    let result = match op {
        "Add" => a.add(b),
        "Sub" => a.sub(b),
        "Mul" => a.mul(b),
        "Div" => {
            let inv = match b.inverse() {
                Ok(m) => m,
                Err(e) => {
                    app.error = Some(format!("求逆失败: {e}"));
                    return;
                }
            };
            a.mul(&inv)
        }
        _ => return,
    };

    match result {
        Ok(m) => {
            let rounded = m.round_to_4();
            let dim = format!("{}×{}", a.rows, b.cols);
            app.history
                .push(format!("A({}×{}) {} B({}×{}) = C({})", a.rows, a.cols, op, b.rows, b.cols, dim));
            app.result = Some(rounded);
            app.focus = Focus::Result;
            app.error = None;
        }
        Err(e) => {
            app.error = Some(format!("运算失败: {e}"));
        }
    }
}

/// [[docs/004-interaction-design.md#43-运算触发交互]]
fn unary_op(app: &mut App, op: &str) {
    let mat = match app.focused_matrix() {
        Some(m) => m,
        None => {
            app.error = Some("矩阵为空".into());
            return;
        }
    };

    if mat.is_empty() {
        app.error = Some("矩阵为空，无法运算".into());
        return;
    }

    let label = app.focus_label().to_string();
    let dim = format!("{}×{}", mat.rows, mat.cols);

    match op {
        "Transpose" => {
            let t = mat.transpose();
            let t_dim = format!("{}×{}", t.rows, t.cols);
            app.history.push(format!("{label}({dim}) = T({t_dim})"));
            app.result = Some(t);
            app.focus = Focus::Result;
            app.error = None;
        }
        "Inverse" => match mat.inverse() {
            Ok(inv) => {
                let rounded = inv.round_to_4();
                app.history.push(format!("inv({label}({dim}))"));
                app.result = Some(rounded);
                app.focus = Focus::Result;
                app.error = None;
            }
            Err(e) => {
                app.error = Some(format!("{e}"));
            }
        },
        "Det" => match mat.det() {
            Ok(d) => {
                app.history.push(format!("det({label}({dim})) = {d:.4}"));
                let result = Matrix::new(1, 1, vec![d]);
                app.result = Some(result);
                app.focus = Focus::Result;
                app.error = None;
            }
            Err(e) => {
                app.error = Some(format!("{e}"));
            }
        },
        "Eigen" => {
            if mat.rows == 2 && mat.cols == 2 {
                match mat.eigenvalues_2x2() {
                    Ok((e1, e2)) => {
                        app.history
                            .push(format!("eigen({label}({dim})) = ({e1:.4}, {e2:.4})"));
                        let result = Matrix::new(2, 1, vec![e1, e2]);
                        app.result = Some(result);
                        app.focus = Focus::Result;
                        app.error = None;
                    }
                    Err(e) => app.error = Some(format!("{e}")),
                }
            } else if mat.rows == 3 && mat.cols == 3 {
                match mat.eigenvalues_3x3() {
                    Ok(ev) => {
                        let ev_str: Vec<String> = ev.iter().map(|v| format!("{v:.4}")).collect();
                        app.history
                            .push(format!("eigen({label}({dim})) = ({})", ev_str.join(", ")));
                        let result = Matrix::new(3, 1, ev);
                        app.result = Some(result);
                        app.focus = Focus::Result;
                        app.error = None;
                    }
                    Err(e) => app.error = Some(format!("{e}")),
                }
            } else {
                app.error = Some("仅支持2×2和3×3矩阵的特征值计算".into());
            }
        }
        _ => {}
    }
}

/// [[docs/004-interaction-design.md#6-文件-io-交互]]
fn handle_file_naming(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char(c) if c.is_ascii_graphic() || c == '.' || c == '-' || c == '_' => {
            app.input_buf.push(c);
        }
        KeyCode::Backspace => {
            app.input_buf.pop();
        }
        KeyCode::Enter => {
            let filename = app.input_buf.clone();
            let is_save = app.file_action == Some(FileAction::Save);
            if is_save {
                if std::path::Path::new(&filename).exists() {
                    app.pending_confirmation = Some("文件已存在, 覆盖? Y/N".into());
                    return;
                }
                app.file_action = None;
                do_save(app, &filename);
            } else {
                app.file_action = None;
                do_load(app, &filename);
            }
        }
        KeyCode::Esc => {
            app.file_action = None;
            app.input_buf.clear();
            app.error = None;
        }
        _ => {}
    }
}

fn do_save(app: &mut App, filename: &str) {
    let mat = match app.focused_matrix() {
        Some(m) => m,
        None => {
            app.error = Some("矩阵为空".into());
            return;
        }
    };
    match mat.save(filename) {
        Ok(()) => {
            app.history.push(format!("保存成功: {filename}"));
            app.error = None;
        }
        Err(e) => app.error = Some(e),
    }
}

fn do_load(app: &mut App, filename: &str) {
    match Matrix::load(filename) {
        Ok(m) => {
            match app.focus {
                Focus::MatrixA => app.mat_a = m,
                Focus::MatrixB => app.mat_b = Some(m),
                Focus::Result => app.result = Some(m),
            }
            app.error = None;
            app.history.push(format!("加载成功: {filename}"));
        }
        Err(e) => {
            app.error = Some(e);
        }
    }
}


