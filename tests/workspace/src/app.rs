use crate::matrix::Matrix;

/// [[docs/002-data-model.md#应用状态]]
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Browsing,
    EditingCell,
    EnteringData,
}

/// [[docs/004-interaction-design.md#6-文件-io-交互]]
#[derive(Debug, Clone, PartialEq)]
pub enum FileAction {
    Save,
    Load,
}

/// [[docs/004-interaction-design.md#31-焦点状态]]
#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    MatrixA,
    MatrixB,
    Result,
}

/// [[docs/002-data-model.md#应用状态]]
pub struct App {
    pub mode: Mode,
    pub mat_a: Matrix,
    pub mat_b: Option<Matrix>,
    pub result: Option<Matrix>,
    pub history: Vec<String>,
    pub error: Option<String>,
    pub selected: (usize, usize),
    pub input_buf: String,
    pub focus: Focus,
    pub show_help: bool,
    pub pending_confirmation: Option<String>,
    pub file_action: Option<FileAction>,
    pub show_history_once: bool,
    pub quit: bool,
}

impl App {
    pub fn new() -> Self {
        App {
            mode: Mode::Browsing,
            mat_a: Matrix::zeros(0, 0),
            mat_b: None,
            result: None,
            history: Vec::new(),
            error: None,
            selected: (0, 0),
            input_buf: String::new(),
            focus: Focus::MatrixA,
            show_help: false,
            pending_confirmation: None,
            file_action: None,
            show_history_once: false,
            quit: false,
        }
    }

    /// [[docs/004-interaction-design.md#13-模式行为矩阵]]
    pub fn focused_matrix(&self) -> Option<&Matrix> {
        match self.focus {
            Focus::MatrixA => Some(&self.mat_a),
            Focus::MatrixB => self.mat_b.as_ref(),
            Focus::Result => self.result.as_ref(),
        }
    }

    pub fn focused_matrix_mut(&mut self) -> Option<&mut Matrix> {
        match self.focus {
            Focus::MatrixA => Some(&mut self.mat_a),
            Focus::MatrixB => self.mat_b.as_mut(),
            Focus::Result => None,
        }
    }

    /// [[docs/004-interaction-design.md#32-焦点视觉效果]]
    pub fn is_focused_empty(&self) -> bool {
        self.focused_matrix().is_none_or(|m| m.is_empty())
    }

    /// [[docs/004-interaction-design.md#32-焦点视觉效果]]
    pub fn focus_label(&self) -> &str {
        match self.focus {
            Focus::MatrixA => "[A]",
            Focus::MatrixB => "[B]",
            Focus::Result => "[结果]",
        }
    }

    pub fn shift_focus(&mut self) {
        self.focus = match self.focus {
            Focus::MatrixA => Focus::MatrixB,
            Focus::MatrixB => Focus::Result,
            Focus::Result => Focus::MatrixA,
        };
        self.selected = (0, 0);
    }

    /// [[docs/004-interaction-design.md#42-矩阵数据输入流程]]
    pub fn clear_focused(&mut self) {
        match self.focus {
            Focus::MatrixA => self.mat_a = Matrix::zeros(0, 0),
            Focus::MatrixB => self.mat_b = None,
            Focus::Result => self.result = None,
        }
    }
}
