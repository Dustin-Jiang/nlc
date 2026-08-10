# 数据模型

核心类型定义，涵盖矩阵结构体、应用状态、错误处理与序列化。

## 矩阵类型

行优先存储，`data[r][c]` 访问第 r 行第 c 列：

```rust
pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<Vec<f64>>,
}
```

## 应用状态

```rust
pub enum Mode {
    Browsing,      // 浏览模式
    EditingCell,   // 编辑单元格
    EnteringData,  // 输入新矩阵
}

pub enum FileAction {
    Save,          // 文件保存
    Load,          // 文件加载
}

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
```

各模式的详细交互流程见 [[docs/004-interaction-design.md#13 模式行为矩阵]]。

实现文件：[[src/app.rs]]

## 错误类型

```rust
pub enum MatrixError {
    DimensionMismatch,   // 维度不匹配
    NotSquare,           // 非方阵
    SingularMatrix,      // 奇异矩阵
    IndexOutOfBounds,    // 索引越界
    Overflow,            // 结果过大
}
```

错误在 UI 层的展示策略见 [[docs/003-ui-design.md#颜色主题]]。

实现文件：[[src/matrix.rs#L333-L339]]

## I/O 序列化

```rust
impl fmt::Display for Matrix { ... }
impl FromStr for Matrix { ... }
pub fn save(&self, path: &str) -> Result<()>
pub fn load(path: &str) -> Result<Matrix>
```

- Display: 列对齐格式化输出
- FromStr: `"1 2 3; 4 5 6; 7 8 9"` → 3×3 矩阵
- save/load: 文件 I/O，配合 [[docs/004-interaction-design.md#6 文件 io 交互]]

实现文件：[[src/matrix.rs#L261-L271]]
