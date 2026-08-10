# 实现计划与测试策略

## Phase 1：矩阵引擎

1. Matrix 结构体、构造函数、基础运算
2. 单元测试覆盖（空矩阵、1×1、奇异矩阵）
3. `cargo test` 全通过

参考 [[docs/002-data-model.md#矩阵类型]] 和 [[docs/005-matrix-operations.md#基础运算]]。

实现文件：[[src/matrix.rs]]

## Phase 2：TUI 骨架

1. 终端初始化、事件循环
2. App 状态机、Mode 枚举
3. 布局分割、MatrixWidget 渲染
4. 键事件解析、基本导航

参考 [[docs/003-ui-design.md#布局结构]] 和 [[docs/004-interaction-design.md]]。

实现文件：[[src/main.rs]] [[src/app.rs]] [[src/ui.rs]]

## Phase 3：交互集成

1. 连接输入 → 状态 → 渲染
2. 矩阵编辑
3. 二元运算流程
4. 错误处理与反馈

参考 [[docs/004-interaction-design.md#13 模式行为矩阵]] 和 [[docs/002-data-model.md#错误类型]]。

实现文件：[[src/input.rs]]

## Phase 4：高级功能

1. 行列式、逆矩阵、特征值
2. 文件保存/加载
3. 历史记录与撤销
4. 帮助系统

参考 [[docs/004-interaction-design.md#5 帮助系统]]。

实现文件：[[src/input.rs#L291-L374]] [[src/ui.rs#L264-L304]]

## 测试策略

### 单元测试

每个运算函数附带 `#[cfg(test)]`：

```rust
#[test]
fn test_add() {
    let a = Matrix::new(2, 2, vec![1.0, 2.0, 3.0, 4.0]);
    let b = Matrix::new(2, 2, vec![5.0, 6.0, 7.0, 8.0]);
    assert_eq!(a.add(&b).unwrap().data[0][0], 6.0);
}
```

### 集成测试

- 端到端流程：创建矩阵 → 运算 → 读取结果
- TUI 渲染用 `ratatui::backend::TestBackend`

### 边界条件

- 空矩阵 → MatrixError
- 维度不匹配 → 拒绝
- 奇异矩阵求逆 → 明确错误消息
