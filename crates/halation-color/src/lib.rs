pub mod display;

#[derive(Debug, Clone)]
pub struct PreparedCurve {
    pub points: Vec<[f32; 2]>,
    pub tangent: Vec<f32>,
}

impl PreparedCurve {
    pub fn new(points: &[[f32; 2]]) -> Self {
        if points.len() < 2 {
            return Self {
                points: points.to_vec(),
                tangent: vec![],
            };
        }
        let n = points.len();
        let slopes: Vec<f32> = points
            .windows(2)
            .map(|p| (p[1][1] - p[0][1]) / (p[1][0] - p[0][0]).max(1e-5))
            .collect();
        let mut tangent = vec![0.0; n];
        tangent[0] = slopes[0];
        tangent[n - 1] = slopes[n - 2];
        for i in 1..n - 1 {
            tangent[i] = if slopes[i - 1] * slopes[i] <= 0.0 {
                0.0
            } else {
                2.0 * slopes[i - 1] * slopes[i] / (slopes[i - 1] + slopes[i])
            };
        }
        Self {
            points: points.to_vec(),
            tangent,
        }
    }

    pub fn sample(&self, x: f32) -> f32 {
        if self.points.len() < 2 {
            return x.clamp(0.0, 1.0);
        }
        let n = self.points.len();
        let i = self
            .points
            .windows(2)
            .position(|p| x <= p[1][0])
            .unwrap_or(n - 2);
        let (x0, y0) = (self.points[i][0], self.points[i][1]);
        let (x1, y1) = (self.points[i + 1][0], self.points[i + 1][1]);
        let h = (x1 - x0).max(1e-5);
        let t = ((x - x0) / h).clamp(0.0, 1.0);
        let t2 = t * t;
        let t3 = t2 * t;
        ((2.0 * t3 - 3.0 * t2 + 1.0) * y0
            + (t3 - 2.0 * t2 + t) * h * self.tangent[i]
            + (-2.0 * t3 + 3.0 * t2) * y1
            + (t3 - t2) * h * self.tangent[i + 1])
            .clamp(0.0, 1.0)
    }
}

pub fn calculate_wheel_tints(grading: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|i| {
        let a = grading[i][0] * std::f32::consts::TAU;
        [a.cos(), (a - 2.094_395_1).cos(), (a + 2.094_395_1).cos()]
    })
}

pub fn apply_curves_and_color_grading(
    rgb: [f32; 3],
    prepared_curves: &[PreparedCurve; 4],
    wheel_tints: &[[f32; 3]; 3],
    grading: &[[f32; 3]; 3],
) -> [f32; 3] {
    let mut out = rgb;
    for c in 0..3 {
        out[c] = prepared_curves[c + 1].sample(out[c]);
    }
    for c in 0..3 {
        out[c] = prepared_curves[0].sample(out[c]);
    }
    let luma = 0.2126 * out[0] + 0.7152 * out[1] + 0.0722 * out[2];
    let weights = [
        (1.0 - luma / 0.42).clamp(0.0, 1.0),
        (1.0 - ((luma - 0.5).abs() / 0.38)).clamp(0.0, 1.0),
        ((luma - 0.58) / 0.42).clamp(0.0, 1.0),
    ];
    for (wi, grade) in grading.iter().enumerate() {
        let sat = grade[1];
        let lum = grade[2];
        let w = weights[wi];
        for c in 0..3 {
            out[c] += wheel_tints[wi][c] * sat * w * 0.11 + lum * w * 0.12;
        }
    }
    [out[0].clamp(0.0, 1.0), out[1].clamp(0.0, 1.0), out[2].clamp(0.0, 1.0)]
}

pub fn white_balance(rgb: [f32; 3], temp: f32, tint: f32) -> [f32; 3] {
    apply_matrix(white_balance_matrix(temp, tint), rgb)
}

pub fn white_balance_matrix(temperature: f32, tint: f32) -> [[f32; 3]; 3] {
    let t = temperature.clamp(2000.0, 50000.0);
    let (x, base_y) = cct_xy(t);
    let (source_x, source_y) = cct_xy(5500.0);
    let y = (base_y - tint.clamp(-150.0, 150.0) * 0.000_025).clamp(0.05, 0.9);
    let source = [
        source_x / source_y,
        1.0,
        (1.0 - source_x - source_y) / source_y,
    ];
    let destination = [x / y, 1.0, (1.0 - x - y) / y];
    const BRADFORD: [[f32; 3]; 3] = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    const BRADFORD_INV: [[f32; 3]; 3] = [
        [0.9869929, -0.1470543, 0.1599627],
        [0.4323053, 0.5183603, 0.0492912],
        [-0.0085287, 0.0400428, 0.9684867],
    ];
    const RGB_TO_XYZ: [[f32; 3]; 3] = [
        [0.4124564, 0.3575761, 0.1804375],
        [0.2126729, 0.7151522, 0.0721750],
        [0.0193339, 0.1191920, 0.9503041],
    ];
    const XYZ_TO_RGB: [[f32; 3]; 3] = [
        [3.2404542, -1.5371385, -0.4985314],
        [-0.9692660, 1.8760108, 0.0415560],
        [0.0556434, -0.2040259, 1.0572252],
    ];
    let src_lms = mul_vec(BRADFORD, source);
    let dst_lms = mul_vec(BRADFORD, destination);
    let scale = [
        dst_lms[0] / src_lms[0],
        dst_lms[1] / src_lms[1],
        dst_lms[2] / src_lms[2],
    ];
    let mut adapt = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            for k in 0..3 {
                adapt[r][c] += BRADFORD_INV[r][k] * scale[k] * BRADFORD[k][c];
            }
        }
    }
    let xyz_adapt = mul_matrix(adapt, RGB_TO_XYZ);
    mul_matrix(XYZ_TO_RGB, xyz_adapt)
}

fn cct_xy(t: f32) -> (f32, f32) {
    let x = if t <= 4000.0 {
        -0.266_123_9e9 / t.powi(3) - 0.234_358_0e6 / t.powi(2) + 0.877_695_6e3 / t + 0.179_910
    } else {
        -3.025_846_9e9 / t.powi(3) + 2.107_037_9e6 / t.powi(2) + 0.222_634_7e3 / t + 0.240_390
    };
    let y = if t <= 2222.0 {
        -1.106_381_4 * x.powi(3) - 1.348_110_20 * x.powi(2) + 2.185_558_32 * x - 0.202_196_83
    } else if t <= 4000.0 {
        -0.954_947_6 * x.powi(3) - 1.374_185_93 * x.powi(2) + 2.091_370_15 * x - 0.167_488_67
    } else {
        3.081_758_0 * x.powi(3) - 5.873_386_70 * x.powi(2) + 3.751_129_97 * x - 0.370_014_83
    };
    (x, y)
}

pub fn apply_matrix(matrix: [[f32; 3]; 3], rgb: [f32; 3]) -> [f32; 3] {
    mul_vec(matrix, rgb).map(|v| v.max(0.0))
}

fn mul_vec(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

fn mul_matrix(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

pub fn apply_highlights_shadows(rgb: [f32; 3], highlights: f32, shadows: f32) -> [f32; 3] {
    let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    let shadow_mask = (1.0 - (luma / 0.35).min(1.0)).powi(2);
    let highlight_mask = (((luma - 0.65) / 0.35).clamp(0.0, 1.0)).powi(2);
    let shadow_gain = (shadows / 100.0) * 0.5;
    let highlight_gain = (highlights / 100.0) * 0.5;
    let factor = 1.0 + (shadow_gain * shadow_mask) + (highlight_gain * highlight_mask);
    [
        (rgb[0] * factor).max(0.0),
        (rgb[1] * factor).max(0.0),
        (rgb[2] * factor).max(0.0),
    ]
}

pub fn apply_whites_blacks(rgb: [f32; 3], whites: f32, blacks: f32) -> [f32; 3] {
    let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    let shadow_mask = (1.0 - (luma / 0.35).clamp(0.0, 1.0)).powi(2);
    let highlight_mask = ((luma - 0.65) / 0.35).clamp(0.0, 1.0).powi(2);
    let white_gain = whites / 100.0 * 0.5 * highlight_mask;
    let black_lift = blacks / 100.0 * 0.05 * shadow_mask;
    [
        (rgb[0] * (1.0 + white_gain) + black_lift).clamp(0.0, 1.0),
        (rgb[1] * (1.0 + white_gain) + black_lift).clamp(0.0, 1.0),
        (rgb[2] * (1.0 + white_gain) + black_lift).clamp(0.0, 1.0),
    ]
}

pub fn apply_contrast(rgb: [f32; 3], contrast: f32) -> [f32; 3] {
    let factor = (259.0 * (contrast + 255.0)) / (255.0 * (259.0 - contrast));
    [
        ((rgb[0] - 0.5) * factor + 0.5).clamp(0.0, 1.0),
        ((rgb[1] - 0.5) * factor + 0.5).clamp(0.0, 1.0),
        ((rgb[2] - 0.5) * factor + 0.5).clamp(0.0, 1.0),
    ]
}