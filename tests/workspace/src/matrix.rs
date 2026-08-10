#![allow(dead_code)]

use std::fmt;
use std::str::FromStr;

/// [[docs/001-architecture.md#整体架构]]
const MAX_DIM: usize = 10;

/// 行优先存储，`data[r][c]` 访问第 r 行第 c 列。
/// [[docs/002-data-model.md#矩阵类型]]
pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<Vec<f64>>,
}

impl Matrix {
    pub fn new(rows: usize, cols: usize, data: Vec<f64>) -> Self {
        assert!(rows <= MAX_DIM && cols <= MAX_DIM, "矩阵维度不能超过 {MAX_DIM}×{MAX_DIM}");
        assert_eq!(data.len(), rows * cols);
        let mut d = Vec::with_capacity(rows);
        for r in 0..rows {
            d.push(data[r * cols..(r + 1) * cols].to_vec());
        }
        Matrix { rows, cols, data: d }
    }

    pub fn zeros(rows: usize, cols: usize) -> Self {
        assert!(rows <= MAX_DIM && cols <= MAX_DIM, "矩阵维度不能超过 {MAX_DIM}×{MAX_DIM}");
        Matrix {
            rows,
            cols,
            data: vec![vec![0.0; cols]; rows],
        }
    }

    pub fn identity(n: usize) -> Self {
        assert!(n <= MAX_DIM, "矩阵维度不能超过 {MAX_DIM}×{MAX_DIM}");
        let mut m = Self::zeros(n, n);
        for i in 0..n {
            m.data[i][i] = 1.0;
        }
        m
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0 || self.cols == 0
    }

    fn check_dim(&self, other: &Matrix) -> Result<(), MatrixError> {
        if self.rows != other.rows || self.cols != other.cols {
            Err(MatrixError::DimensionMismatch)
        } else {
            Ok(())
        }
    }

    fn check_square(&self) -> Result<(), MatrixError> {
        if self.rows != self.cols {
            Err(MatrixError::NotSquare)
        } else {
            Ok(())
        }
    }

    /// [[docs/005-matrix-operations.md#基础运算]]
    pub fn add(&self, other: &Matrix) -> Result<Matrix, MatrixError> {
        self.check_dim(other)?;
        let data: Vec<f64> = self
            .data
            .iter()
            .flatten()
            .zip(other.data.iter().flatten())
            .map(|(a, b)| a + b)
            .collect();
        Ok(Matrix::new(self.rows, self.cols, data))
    }

    pub fn sub(&self, other: &Matrix) -> Result<Matrix, MatrixError> {
        self.check_dim(other)?;
        let data: Vec<f64> = self
            .data
            .iter()
            .flatten()
            .zip(other.data.iter().flatten())
            .map(|(a, b)| a - b)
            .collect();
        Ok(Matrix::new(self.rows, self.cols, data))
    }

    pub fn mul(&self, other: &Matrix) -> Result<Matrix, MatrixError> {
        if self.cols != other.rows {
            return Err(MatrixError::DimensionMismatch);
        }
        let mut data = vec![0.0_f64; self.rows * other.cols];
        for i in 0..self.rows {
            for j in 0..other.cols {
                let mut sum = 0.0;
                for k in 0..self.cols {
                    sum += self.data[i][k] * other.data[k][j];
                }
                data[i * other.cols + j] = sum;
            }
        }
        Ok(Matrix::new(self.rows, other.cols, data))
    }

    pub fn scalar_mul(&self, k: f64) -> Matrix {
        let data: Vec<f64> = self.data.iter().flatten().map(|v| v * k).collect();
        Matrix::new(self.rows, self.cols, data)
    }

    pub fn hadamard(&self, other: &Matrix) -> Result<Matrix, MatrixError> {
        self.check_dim(other)?;
        let data: Vec<f64> = self
            .data
            .iter()
            .flatten()
            .zip(other.data.iter().flatten())
            .map(|(a, b)| a * b)
            .collect();
        Ok(Matrix::new(self.rows, self.cols, data))
    }

    /// [[docs/005-matrix-operations.md#高级运算]]
    pub fn transpose(&self) -> Matrix {
        let mut data = vec![0.0_f64; self.rows * self.cols];
        for r in 0..self.rows {
            for c in 0..self.cols {
                data[c * self.rows + r] = self.data[r][c];
            }
        }
        Matrix::new(self.cols, self.rows, data)
    }

    pub fn det(&self) -> Result<f64, MatrixError> {
        self.check_square()?;
        let n = self.rows;
        if n == 0 {
            return Err(MatrixError::DimensionMismatch);
        }
        if n == 1 {
            return Ok(self.data[0][0]);
        }
        if n == 2 {
            return Ok(self.data[0][0] * self.data[1][1] - self.data[0][1] * self.data[1][0]);
        }
        let mut det = 0.0;
        for j in 0..n {
            let cof = self.cofactor(0, j);
            det += self.data[0][j] * cof.det().unwrap_or(0.0) * if j % 2 == 0 { 1.0 } else { -1.0 };
        }
        Ok(det)
    }

    pub fn cofactor(&self, row: usize, col: usize) -> Matrix {
        let mut data = Vec::new();
        for r in 0..self.rows {
            if r == row {
                continue;
            }
            for c in 0..self.cols {
                if c == col {
                    continue;
                }
                data.push(self.data[r][c]);
            }
        }
        Matrix::new(self.rows - 1, self.cols - 1, data)
    }

    pub fn adjugate(&self) -> Result<Matrix, MatrixError> {
        self.check_square()?;
        let n = self.rows;
        let mut data = vec![0.0_f64; n * n];
        for i in 0..n {
            for j in 0..n {
                let cof = self.cofactor(i, j);
                let val = cof.det().unwrap_or(0.0);
                data[j * n + i] = if (i + j) % 2 == 0 { val } else { -val };
            }
        }
        Ok(Matrix::new(n, n, data))
    }

    pub fn inverse(&self) -> Result<Matrix, MatrixError> {
        let d = self.det()?;
        if d.abs() < 1e-10 {
            return Err(MatrixError::SingularMatrix);
        }
        let adj = self.adjugate()?;
        Ok(adj.scalar_mul(1.0 / d))
    }

    pub fn trace(&self) -> Result<f64, MatrixError> {
        self.check_square()?;
        Ok((0..self.rows).map(|i| self.data[i][i]).sum())
    }

    pub fn eigenvalues_2x2(&self) -> Result<(f64, f64), MatrixError> {
        self.check_square()?;
        if self.rows != 2 {
            return Err(MatrixError::DimensionMismatch);
        }
        let a = self.data[0][0];
        let b = self.data[0][1];
        let c = self.data[1][0];
        let d = self.data[1][1];
        let trace = a + d;
        let det = a * d - b * c;
        let disc = trace * trace - 4.0 * det;
        if disc < 0.0 {
            return Err(MatrixError::Overflow);
        }
        let sqrt_disc = disc.sqrt();
        Ok(((trace + sqrt_disc) / 2.0, (trace - sqrt_disc) / 2.0))
    }

    pub fn eigenvalues_3x3(&self) -> Result<Vec<f64>, MatrixError> {
        self.check_square()?;
        if self.rows != 3 {
            return Err(MatrixError::DimensionMismatch);
        }
        let a = self.data[0][0];
        let b = self.data[0][1];
        let c = self.data[0][2];
        let d = self.data[1][0];
        let e = self.data[1][1];
        let f = self.data[1][2];
        let g = self.data[2][0];
        let h = self.data[2][1];
        let i = self.data[2][2];

        let tr = a + e + i;
        let q = (a * e + a * i + e * i - b * d - c * g - f * h) / 3.0;
        let r = (a * e * i + 2.0 * b * f * g - a * f * f - e * c * c - i * b * b) / 2.0;

        let p = tr * tr / 9.0 - q;
        let r2 = r * r;
        let p3 = p * p * p;

        if r2 <= p3 {
            let theta = (r / p.sqrt().max(1e-30)).acos();
            let sqrt_p = p.sqrt() * 2.0;
            let t3 = tr / 3.0;
            Ok(vec![
                Self::round4(sqrt_p * (theta / 3.0).cos() + t3),
                Self::round4(sqrt_p * ((theta + 2.0 * std::f64::consts::PI) / 3.0).cos() + t3),
                Self::round4(sqrt_p * ((theta + 4.0 * std::f64::consts::PI) / 3.0).cos() + t3),
            ])
        } else {
            Err(MatrixError::SingularMatrix)
        }
    }

    fn round4(v: f64) -> f64 {
        (v * 10000.0).round() / 10000.0
    }

    /// 四舍五入到 4 位小数
    pub fn round_to_4(&self) -> Matrix {
        let data: Vec<f64> = self.data.iter().flatten().map(|v| Self::round4(*v)).collect();
        Matrix::new(self.rows, self.cols, data)
    }

    /// [[docs/002-data-model.md#io-序列化]]
    pub fn save(&self, path: &str) -> Result<(), String> {
        let s = self.to_string();
        std::fs::write(path, s).map_err(|e| format!("保存失败: {e}"))
    }

    pub fn load(path: &str) -> Result<Matrix, String> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;
        s.parse()
    }
}

/// [[docs/002-data-model.md#io-序列化]]
impl fmt::Display for Matrix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (r, row) in self.data.iter().enumerate() {
            if r > 0 {
                writeln!(f)?;
            }
            for (c, val) in row.iter().enumerate() {
                if c > 0 {
                    write!(f, " ")?;
                }
                write!(f, "{:8.4}", val)?;
            }
        }
        Ok(())
    }
}

/// [[docs/002-data-model.md#io-序列化]]
/// `"1 2 3; 4 5 6; 7 8 9"` → 3×3 矩阵
impl FromStr for Matrix {
    type Err = String;

    fn from_str(s: &str) -> Result<Matrix, String> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("空矩阵".into());
        }
        let rows_str: Vec<&str> = trimmed.split(';').map(|s| s.trim()).collect();
        if rows_str.is_empty() {
            return Err("空矩阵".into());
        }
        let cols = rows_str[0]
            .split_whitespace()
            .map(|s| s.parse::<f64>().map_err(|_| format!("无法解析数字: {s}")))
            .collect::<Result<Vec<f64>, String>>()?
            .len();
        if cols == 0 {
            return Err("空行或空矩阵".into());
        }
        let mut data = Vec::new();
        for (ri, row_str) in rows_str.iter().enumerate() {
            let row: Vec<f64> = row_str
                .split_whitespace()
                .map(|s| s.parse::<f64>().map_err(|_| format!("位置({},1): 无法解析\"{s}\"为数字", ri + 1)))
                .collect::<Result<Vec<f64>, String>>()?;
            if row.len() != cols {
                return Err(format!(
                    "行{}列数不匹配: 期待{cols}列, 得到{}列",
                    ri + 1,
                    row.len()
                ));
            }
            data.extend(row);
        }
        Ok(Matrix::new(rows_str.len(), cols, data))
    }
}

/// [[docs/002-data-model.md#错误类型]]
#[derive(Debug, Clone, PartialEq)]
pub enum MatrixError {
    DimensionMismatch,
    NotSquare,
    SingularMatrix,
    IndexOutOfBounds,
    Overflow,
}

impl fmt::Display for MatrixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MatrixError::DimensionMismatch => write!(f, "维度不匹配"),
            MatrixError::NotSquare => write!(f, "非方阵"),
            MatrixError::SingularMatrix => write!(f, "奇异矩阵"),
            MatrixError::IndexOutOfBounds => write!(f, "索引越界"),
            MatrixError::Overflow => write!(f, "结果过大"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let m = Matrix::new(2, 3, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(m.rows, 2);
        assert_eq!(m.cols, 3);
        assert_eq!(m.data[0][1], 2.0);
    }

    #[test]
    fn test_add() {
        let a = Matrix::new(2, 2, vec![1.0, 2.0, 3.0, 4.0]);
        let b = Matrix::new(2, 2, vec![5.0, 6.0, 7.0, 8.0]);
        let c = a.add(&b).unwrap();
        assert_eq!(c.data[0][0], 6.0);
        assert_eq!(c.data[1][1], 12.0);
    }

    #[test]
    fn test_add_dim_mismatch() {
        let a = Matrix::new(2, 2, vec![1.0; 4]);
        let b = Matrix::new(2, 3, vec![1.0; 6]);
        assert!(a.add(&b).is_err());
    }

    #[test]
    fn test_mul() {
        let a = Matrix::new(2, 3, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let b = Matrix::new(3, 2, vec![7.0, 8.0, 9.0, 10.0, 11.0, 12.0]);
        let c = a.mul(&b).unwrap();
        assert_eq!(c.rows, 2);
        assert_eq!(c.cols, 2);
        assert!((c.data[0][0] - 58.0).abs() < 1e-10);
    }

    #[test]
    fn test_transpose() {
        let a = Matrix::new(2, 3, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let t = a.transpose();
        assert_eq!(t.rows, 3);
        assert_eq!(t.cols, 2);
        assert_eq!(t.data[0][0], 1.0);
        assert_eq!(t.data[1][1], 5.0);
    }

    #[test]
    fn test_det_2x2() {
        let a = Matrix::new(2, 2, vec![1.0, 2.0, 3.0, 4.0]);
        assert!((a.det().unwrap() - (-2.0)).abs() < 1e-10);
    }

    #[test]
    fn test_det_3x3() {
        let a = Matrix::new(3, 3, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0]);
        assert!((a.det().unwrap() - (-3.0)).abs() < 1e-10);
    }

    #[test]
    fn test_inverse_2x2() {
        let a = Matrix::new(2, 2, vec![4.0, 7.0, 2.0, 6.0]);
        let inv = a.inverse().unwrap();
        let eye = a.mul(&inv).unwrap().round_to_4();
        assert!((eye.data[0][0] - 1.0).abs() < 1e-4);
        assert!((eye.data[1][1] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_singular_det() {
        let a = Matrix::new(2, 2, vec![1.0, 2.0, 2.0, 4.0]);
        assert!(a.inverse().is_err());
    }

    #[test]
    fn test_display() {
        let a = Matrix::new(2, 2, vec![1.0, 2.0, 3.0, 4.0]);
        let s = a.to_string();
        assert!(s.contains("1.0000"));
        assert!(s.contains("4.0000"));
    }

    #[test]
    fn test_from_str() {
        let m: Matrix = "1 2 3; 4 5 6; 7 8 9".parse().unwrap();
        assert_eq!(m.rows, 3);
        assert_eq!(m.cols, 3);
        assert_eq!(m.data[0][0], 1.0);
        assert_eq!(m.data[2][2], 9.0);
    }

    #[test]
    fn test_from_str_bad_cols() {
        let r: Result<Matrix, _> = "1 2; 3".parse();
        assert!(r.is_err());
    }

    #[test]
    fn test_scalar_mul() {
        let a = Matrix::new(2, 2, vec![1.0, 2.0, 3.0, 4.0]);
        let b = a.scalar_mul(2.0);
        assert_eq!(b.data[0][0], 2.0);
        assert_eq!(b.data[1][1], 8.0);
    }

    #[test]
    fn test_hadamard() {
        let a = Matrix::new(2, 2, vec![1.0, 2.0, 3.0, 4.0]);
        let b = Matrix::new(2, 2, vec![5.0, 6.0, 7.0, 8.0]);
        let c = a.hadamard(&b).unwrap();
        assert_eq!(c.data[0][0], 5.0);
        assert_eq!(c.data[1][1], 32.0);
    }

    #[test]
    fn test_trace() {
        let a = Matrix::new(3, 3, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        assert!((a.trace().unwrap() - 15.0).abs() < 1e-10);
    }

    #[test]
    fn test_identity() {
        let i = Matrix::identity(3);
        assert_eq!(i.data[0][0], 1.0);
        assert_eq!(i.data[1][1], 1.0);
        assert_eq!(i.data[2][2], 1.0);
        assert_eq!(i.data[0][1], 0.0);
    }

    #[test]
    fn test_empty() {
        let m = Matrix::zeros(0, 0);
        assert!(m.is_empty());
    }

    #[test]
    fn test_not_square() {
        let a = Matrix::new(2, 3, vec![1.0; 6]);
        assert!(a.det().is_err());
        assert!(a.inverse().is_err());
    }

    #[test]
    fn test_eigenvalues_2x2() {
        let a = Matrix::new(2, 2, vec![2.0, 0.0, 0.0, 3.0]);
        let (e1, e2) = a.eigenvalues_2x2().unwrap();
        assert!((e1 - 3.0).abs() < 1e-4 || (e1 - 2.0).abs() < 1e-4);
        assert!((e2 - 3.0).abs() < 1e-4 || (e2 - 2.0).abs() < 1e-4);
    }

    #[test]
    fn test_adjugate() {
        let a = Matrix::new(2, 2, vec![1.0, 2.0, 3.0, 4.0]);
        let adj = a.adjugate().unwrap();
        assert_eq!(adj.data[0][0], 4.0);
        assert_eq!(adj.data[1][1], 1.0);
        assert_eq!(adj.data[0][1], -2.0);
        assert_eq!(adj.data[1][0], -3.0);
    }

    #[test]
    fn test_from_str_empty() {
        assert!("".parse::<Matrix>().is_err());
    }

    #[test]
    fn test_eigenvalue_not_square() {
        let a = Matrix::new(2, 3, vec![1.0; 6]);
        assert!(a.eigenvalues_2x2().is_err());
        assert!(a.eigenvalues_3x3().is_err());
    }

    #[test]
    fn test_mul_dim_mismatch() {
        let a = Matrix::new(2, 2, vec![1.0; 4]);
        let b = Matrix::new(3, 3, vec![1.0; 9]);
        assert!(a.mul(&b).is_err());
    }
}
