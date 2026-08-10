# 交互设计

详细定义用户操作状态机、焦点管理、输入事件流、反馈机制与错误恢复策略。

## 1. 模式状态机

### 1.1 模式枚举

```rust
pub enum Mode {
    Browsing,
    EditingCell,
    EnteringData,
}
```

### 1.2 状态转换图

```
                 ┌──────────────────────────────────┐
                 │                                  │
                 ▼                                  │
         ┌──────────────┐     Enter     ┌──────────────┐
         │              │──────────────>│              │
         │   Browsing   │               │ EditingCell  │
         │              │<──────────────│              │
         └──────┬───────┘     Esc       └──────────────┘
            │   │   │                          │
       Tab  │   │   │ Enter(新矩阵)             │
            │   │   ▼                          │
            │   │ ┌──────────────┐             │
            │   │ │EnteringData  │             │
            │   │ └──────────────┘             │
            │   │       │                      │
            │   │    Esc/Enter(完成)            │
            │   │       │                      │
            ▼   ▼       ▼                      ▼
         [所有模式: Q → 退出程序]
```

### 1.3 模式行为矩阵

| 模式 | 允许的输入 | 渲染重点 | 状态栏提示 |
|------|-----------|---------|-----------|
| Browsing | 导航键 + 操作快捷键 | 矩阵网格 + 操作栏 | `Tab切换 ↑↓←→移动 Enter编辑 +-*/运算` |
| EditingCell | 数字 + `.` + `-` + Enter/Esc | 选中单元格高亮闪烁 | `输入数值 · Enter确认 Esc取消` |
| EnteringData | 数字 + `;` + Enter/Esc | 输入缓冲区实时预览 | `行用;分隔 · Enter完成 Esc取消` |

### 1.4 模式切换规则

**Browsing → EditingCell：**
- 条件：按下 Enter，且焦点在 MatrixA 或 MatrixB
- 动作：选中单元格进入编辑，复制当前值到 input_buf
- 异常：空矩阵无法编辑 → 保持 Browsing，状态栏显示错误 `矩阵为空，无法编辑`

**Browsing → EnteringData：**
- 条件：按下 Enter，且焦点在空矩阵（rows = 0）或按下 c 清空后
- 动作：清空 input_buf，等待用户输入矩阵数据
- 视觉：矩阵区域显示输入缓冲区内容

**EditingCell → Browsing：**
- 条件：Enter（确认）或 Esc（取消）
- Enter 动作：解析 input_buf 为 f64，更新矩阵单元格。解析失败 → 保持 EditingCell，错误闪烁
- Esc 动作：丢弃 input_buf，恢复原值

**EnteringData → Browsing：**
- 条件：Enter（完成输入）或 Esc（取消）
- Enter 动作：解析 input_buf 为 Matrix。格式错误 → 保持 EnteringData，错误描述
- Esc 动作：丢弃输入，矩阵恢复为空

## 2. 输入事件流

### 2.1 事件处理管线

```
crossterm::event::read()
    │
    ▼
┌─────────────────────┐
│  事件分类            │
│  KeyEvent | Resize   │
└─────────┬───────────┘
          │ KeyEvent
          ▼
┌─────────────────────┐     Mode::Browsing
│  dispatch_by_mode()  │────────────────────>
│                      │     Mode::EditingCell
│  按当前 Mode 分发     │────────────────────>
│                      │     Mode::EnteringData
└─────────────────────┘────────────────────>
          │
          ▼
┌─────────────────────┐
│  update_app_state()  │
│  更新 App 字段       │
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│  handle_tick()       │
│  触发重绘            │
└─────────────────────┘
```

### 2.2 按键分发表

#### Browsing 模式

| 按键 | 处理器 | 副作用 |
|------|--------|--------|
| `Tab` | shift_focus() | 焦点在 A → B → 结果 → A 循环 |
| `↑` | move_selection(-1, 0) | 边界卡住 |
| `↓` | move_selection(1, 0) | 边界卡住 |
| `←` | move_selection(0, -1) | 边界卡住 |
| `→` | move_selection(0, 1) | 边界卡住 |
| `Enter` | start_editing() | 模式切换 |
| `+` | binary_op(Add) | A + B → result |
| `-` | binary_op(Sub) | A - B → result |
| `*` | binary_op(Mul) | A × B → result |
| `/` | binary_op(Div) | A ÷ B → result |
| `t` | unary_op(Transpose) | 转置焦点矩阵 |
| `i` | unary_op(Inverse) | 求逆焦点矩阵 |
| `d` | unary_op(Det) | det(焦点矩阵) → result |
| `e` | unary_op(Eigen) | 特征值 → result |
| `c` | clear_focused() | 清空后进入 EnteringData |
| `s` | save_matrix() | 保存焦点矩阵 |
| `l` | load_matrix() | 加载替换焦点矩阵 |
| `h` | toggle_help() | 显示/隐藏帮助 |
| `Esc` | confirm_quit() | 退出确认对话框 |
| `q` | confirm_quit() | 退出确认对话框 |
| `Ctrl-C` | confirm_quit() | 全局退出确认 |

#### EditingCell 模式

| 按键 | 处理器 | 副作用 |
|------|--------|--------|
| `0-9` | append_digit(c) | input_buf 追加字符 |
| `.` | append_dot() | 已含 `.` 则忽略 |
| `-` | toggle_sign() | 切换正负号 |
| `Backspace` | pop_char() | 删除末尾字符 |
| `Enter` | commit_edit() | 验证 → 更新/拒绝 |
| `Esc` | cancel_edit() | 恢复原值 |

#### EnteringData 模式

| 按键 | 处理器 | 副作用 |
|------|--------|--------|
| `0-9 . -` | append_char(c) | input_buf 追加 |
| `;` | end_row() | 当前行结束，追加 `;` |
| `Backspace` | pop_char() | 删除末尾字符 |
| `Enter` | commit_matrix() | 解析 → 创建/拒绝 |
| `Esc` | cancel_input() | 丢弃所有输入 |

### 2.3 无效输入处理

任何模式下收到未绑定的按键：

```
unhandled_key(key)
    → status_bar 闪烁显示 "无效按键" (300ms 自动消失)
    → 不改变任何状态
    → 不触发重绘（节省性能）
```

## 3. 焦点系统

### 3.1 焦点状态

```rust
pub enum Focus {
    MatrixA,
    MatrixB,
    Result,
}
```

三焦点循环：A → B → Result → A。Result 为只读焦点（不能编辑，但可以作为运算目标选择）。

### 3.2 焦点视觉效果

| 焦点 | MatrixWidget 边框 | 状态栏指示 |
|------|------------------|-----------|
| MatrixA | White Bold | `[A] Tab切换` |
| MatrixB | White Bold | `[B] Tab切换` |
| Result | Dim | `[结果] 只读` |

### 3.3 空焦点状态

当所有矩阵均为空（初始启动），Tab 循环仍有效，但 Enter 直接进入 EnteringData 而非 EditingCell。

## 4. 编辑交互

### 4.1 单元格编辑流程

```
用户按下 Enter（Browsing 模式，焦点在 MatrixA）
    │
    ├─ 矩阵为空？── Yes → 进入 EnteringData 模式
    │
    └─ No
         │
         ▼
    复制 selected 处值到 input_buf
    mode ← EditingCell
    input_buf ← format!("{}", matrix[selected])
    status_bar ← "输入数值 · Enter确认 Esc取消"
         │
         ▼
    用户输入数字字符 → input_buf.append()
         │
         ▼
    用户按下 Enter
         │
         ├─ input_buf 解析为 f64？── Yes → 更新矩阵，mode ← Browsing
         │
         └─ No
              │
              ▼
          input_buf 闪烁红色（500ms）
          status_bar ← "无效数字"
          保持 EditingCell
```

### 4.2 矩阵数据输入流程

```
用户按下 Enter（空矩阵焦点）
  或按下 c（清空焦点矩阵）
    │
    ▼
mode ← EnteringData
input_buf ← ""
status_bar ← "行用;分隔 如: 1 2 3; 4 5 6; 7 8 9"
    │
    ▼
用户输入字符构建矩阵字符串
    │
    ▼
用户按下 Enter
    │
    ├─ 解析成功 (rows × cols > 0)
    │   ├─ 替换焦点矩阵
    │   ├─ mode ← Browsing
    │   └─ status_bar ← "矩阵 {rows}×{cols} 加载成功"
    │
    └─ 解析失败
        ├─ input_buf 保留（可编辑修正）
        ├─ status_bar ← "解析失败: {错误描述}"
        └─ 保持 EnteringData
```

解析错误示例：

| 输入 | 错误消息 |
|------|---------|
| `1 2; 3` | `行2列数不匹配: 期待2列, 得到1列` |
| `abc` | `位置(1,1): 无法解析"abc"为数字` |
| `; ;` | `空行或空矩阵` |

### 4.3 运算触发交互

```
用户按下 +（Browsing 模式）
    │
    ├─ A 或 B 为空？── Yes → status_bar ← "矩阵为空，无法运算"
    │
    ├─ 维度不匹配？── Yes → status_bar ← "维度不匹配: A {a}×{a2}, B {b}×{b2}"
    │
    └─ No
         │
         ▼
    调用 matrix::add(A, B)
         │
         ├─ Ok(result) → 更新 App.result，焦点切到 Result
         │
         └─ Err(e)
              ├─ status_bar ← 错误描述
              └─ 保持当前模式
```

### 4.4 破坏性操作确认

涉及数据丢失的操作需要二次确认，使用 [[docs/003-ui-design.md#dialog]] Dialog 组件库部件弹出覆盖层。

实现文件：[[src/input.rs#L31-L52]]

| 操作 | 确认方式 |
|------|---------|
| 清空矩阵 (`c`) | 仅非空矩阵需要确认：Dialog 显示 `确认清空? Y/N` → 按 Y 执行，N 取消 |
| 退出程序 (`q`/`Esc`/`Ctrl-C`) | Dialog 显示 `确认退出? Y/N` |
| 覆盖加载 (`l`) | 仅非空矩阵需要确认：Dialog 显示 `文件将覆盖当前矩阵, 确认? Y/N` |

确认流程：

```
用户按 q / Esc / Ctrl-C
    │
    ▼
App.quit = false
pending_confirmation ← Some("确认退出? Y/N")
render()  // Dialog 覆盖层弹出
    │
    ▼
等待输入（Dialog 打开时所有按键仅 Dialog 处理）：
    Y/y → App.quit = true → 主循环退出
    N/n 或 Esc → pending_confirmation = None → Dialog 关闭
    其他 → 忽略
```

## 5. 帮助系统

### 5.1 帮助覆盖层

按下 `h` 在任何模式切换帮助覆盖层显示。帮助层覆盖矩阵区域，包含：

```
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
│  系统                         │
│  H         切换此帮助          │
│  Q/Esc     退出程序            │
│  Ctrl-C    全局退出（任何模式） │
└──────────────────────────────┘
```

### 5.2 帮助行为

- 帮助覆盖时，所有输入都无效（除 `h`、`q`、`Esc`、`Ctrl-C`）
- 再次按 `h` 关闭帮助
- 按 `q`/`Esc` 在帮助打开时仍然退出

### 5.3 上下文提示

状态栏第一段始终显示当前可用的主要操作：

| 模式 | 提示文本 |
|------|---------|
| Browsing | `Tab切换 ↑↓←→移动 Enter编辑 +-*/运算` |
| EditingCell | `输入数值 · Enter确认 Esc取消` |
| EnteringData | `行用;分隔 · Enter完成 Esc取消` |
| 确认提示 | `确认? Y/N` |

## 6. 文件 I/O 交互

### 6.1 保存流程

```
用户按下 s
    │
    ├─ 焦点矩阵为空？── Yes → status_bar ← "矩阵为空，无法保存"
    │
    └─ No
         │
         ▼
    进入文件命名模式（简化版）
    status_bar ← "输入文件名:"
    input_buf ← "matrix_1.txt"
    mode ← Browsing（保持）
    
    用户输入文件名后按 Enter
         │
         ├─ 文件已存在？
         │   ├─ Yes → status_bar ← "文件已存在, 覆盖? Y/N"
         │   │    Y → 写入覆盖
         │   │    N → 取消
         │   └─ No → 写入新文件
         │
         status_bar ← "保存成功: {filename}"
         │
         └─ I/O 错误 → status_bar ← "保存失败: {错误}"
```

### 6.2 加载流程

```
用户按下 l
    │
    ├─ 焦点矩阵非空？
    │   ├─ Yes → 确认覆盖提示
    │   └─ No → 直接进入选择
    │
    ▼
    status_bar ← "输入文件名:"
    input_buf 显示默认路径
    
    用户输入文件名后按 Enter
         ├─ 文件不存在 → status_bar ← "文件不存在"
         ├─ 解析失败 → status_bar ← "文件格式错误: {描述}"
         └─ 成功 → 替换焦点矩阵, status_bar ← "加载成功"
```

## 7. 操作历史

### 7.1 历史记录

```rust
pub struct App {
    // ...
    pub history: Vec<String>,
}
```

每次成功运算在 history 追加一条记录。格式：

| 运算 | 历史记录 |
|------|---------|
| A + B | `A(2×3) + B(2×3) = C(2×3)` |
| det(A) | `det(A(3×3)) = 6.0000` |
| 编辑单元格 | `A[1,2]: 0 → 5` |
| 转置 | `A(3×2) = T(2×3)` |

### 7.2 历史查看

绑定 `u` 键切换历史显示：

```
按下 u
    │
    ├─ history 为空？→ status_bar ← "暂无操作历史"
    │
    └─ 在状态栏轮播显示最近 3 条历史
```

## 8. 错误恢复

### 8.1 错误显示策略

| 错误类型 | 严重度 | 显示方式 | 自动消失 |
|---------|--------|---------|---------|
| 无效按键 | 提示 | 状态栏闪烁 | 300ms |
| 空矩阵运算 | 提示 | 状态栏文本 | 下次按键 |
| 维度不匹配 | 警告 | 状态栏文本 + 描述 | 下次按键 |
| 奇异矩阵 | 警告 | 状态栏文本 | 下次按键 |
| 输入解析失败 | 错误 | 状态栏 + 输入保留 | 手动修正 |
| 文件 I/O 错误 | 错误 | 状态栏文本 | 下次按键 |

### 8.2 错误恢复路径

```
运算错误发生
    │
    ▼
status_bar ← 错误描述
error 字段设置
    │
    ├─ 严重错误（I/O 等）
    │   → 保持当前模式
    │   → 下次按键清除 error
    │   → 用户修正后重试
    │
    └─ 输入错误（解析失败）
        → 保持当前编辑模式
        → input_buf 保留，可继续编辑
        → 用户修正后按 Enter 重试
```

### 8.3 不可恢复状态

以下情况应 panic 退出（属于编程错误，非用户操作）：

- ratatui 终端初始化失败
- crossterm 进入 raw mode 失败
- 内存分配失败（10×10 矩阵不应触发，但需防御）

## 9. 边界情况

### 9.1 启动边界

- 首次启动：矩阵 A 显示为 `[空] 3×3 占位`，B 和 Result 隐藏
- 提示：`按 H 查看帮助 · 按 Enter 输入矩阵`

### 9.2 终端尺寸不足

- 最小终端尺寸：80×24
- 小于最小值时：状态栏显示 `终端太小，请放大至 80×24 以上`
- 不渲染矩阵内容，仅显示提示

### 9.3 连续快速输入

- 按键事件排队由 crossterm 处理
- 每帧处理一个事件，不累积
- 渲染频率由 ratatui 的帧率控制

### 9.4 中段中断

- 程序退出前 restore terminal（crossterm::terminal::disable_raw_mode）
- 通过 Drop trait 或 panic hook 保证
