pub fn logo_rgba(size: usize) -> Vec<u8> {
    let mut rgba = vec![0; size * size * 4];
    let curves = [
        [[0.71, 0.30], [0.62, 0.16], [0.29, 0.18], [0.29, 0.36]],
        [[0.29, 0.36], [0.29, 0.47], [0.43, 0.48], [0.52, 0.50]],
        [[0.52, 0.50], [0.65, 0.53], [0.73, 0.56], [0.70, 0.67]],
        [[0.70, 0.67], [0.67, 0.85], [0.36, 0.83], [0.27, 0.70]],
    ];
    let mut points: Vec<[f32; 2]> = Vec::new();
    for curve in curves {
        for i in 0..=64 {
            let t = i as f32 / 64.0;
            let u = 1.0 - t;
            points.push(std::array::from_fn(|axis| {
                u * u * u * curve[0][axis]
                    + 3.0 * u * u * t * curve[1][axis]
                    + 3.0 * u * t * t * curve[2][axis]
                    + t * t * t * curve[3][axis]
            }));
        }
    }
    for y in 0..size {
        for x in 0..size {
            let fx = (x as f32 + 0.5) / size as f32;
            let fy = (y as f32 + 0.5) / size as f32;
            let dx = (fx - 0.5).abs() - 0.28;
            let dy = (fy - 0.5).abs() - 0.28;
            if dx.max(0.0).hypot(dy.max(0.0)) > 0.18 {
                continue;
            }
            let distance = points
                .iter()
                .map(|p| (fx - p[0]).hypot(fy - p[1]))
                .fold(f32::INFINITY, f32::min);
            let ink = ((0.042 - distance) * size as f32 + 0.5).clamp(0.0, 1.0);
            let foreground = [17.0, 22.0, 29.0];
            let background = [245.0, 158.0, 66.0];
            let color = [
                (background[0] * (1.0 - ink) + foreground[0] * ink) as u8,
                (background[1] * (1.0 - ink) + foreground[1] * ink) as u8,
                (background[2] * (1.0 - ink) + foreground[2] * ink) as u8,
                255,
            ];
            rgba[(y * size + x) * 4..(y * size + x + 1) * 4].copy_from_slice(&color);
        }
    }
    rgba
}
