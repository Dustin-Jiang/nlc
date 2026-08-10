# 矩阵运算

纯函数式 API，所有运算不修改原矩阵，返回新矩阵或错误。

## 基础运算

| 运算 | 签名 |
|------|------|
| 加法 | `add(&self, other: &Matrix) -> Result<Matrix>` |
| 减法 | `sub(&self, other: &Matrix) -> Result<Matrix>` |
| 乘法 | `mul(&self, other: &Matrix) -> Result<Matrix>` |
| 标量乘 | `scalar_mul(&self, k: f64) -> Matrix` |
| 逐元素乘 | `hadamard(&self, other: &Matrix) -> Result<Matrix>` |

## 高级运算

| 运算 | 签名 |
|------|------|
| 转置 | `transpose(&self) -> Matrix` |
| 行列式 | `det(&self) -> Result<f64>` |
| 余子式 | `cofactor(&self, row: usize, col: usize) -> Matrix` |
| 伴随矩阵 | `adjugate(&self) -> Result<Matrix>` |
| 逆矩阵 | `inverse(&self) -> Result<Matrix>` |
| 迹 | `trace(&self) -> Result<f64>` |
| 特征值 2×2 | `eigenvalues_2x2(&self) -> Result<(f64, f64)>` |
| 特征值 3×3 | `eigenvalues_3x3(&self) -> Result<Vec<f64>>` |

## 数值稳定性

- 递归余子式求行列式（≤10×10 可接受）
- 伴随矩阵法求逆
- 解析解求特征值（仅 2×2 和 3×3）
- ε = 1e-10 判断奇异矩阵

实现文件：[[src/matrix.rs]]
