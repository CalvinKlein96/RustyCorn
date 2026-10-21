//! Axis ticks: values from `rc_plotdata::nice_ticks` inside the visible range,
//! labelled with `rc_plotdata::tick_labels`.

/// Tick values inside `[lo, hi]`, about `target` of them.
pub fn ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    if !(lo.is_finite() && hi.is_finite()) || hi <= lo {
        return Vec::new();
    }
    let eps = (hi - lo) * 1e-9;
    rc_plotdata::nice_ticks(lo, hi, target)
        .into_iter()
        .filter(|v| *v >= lo - eps && *v <= hi + eps)
        .collect()
}

/// Ticks and their labels.
pub fn labelled(lo: f64, hi: f64, target: usize) -> (Vec<f64>, Vec<String>) {
    let v = ticks(lo, hi, target);
    let mut l = rc_plotdata::tick_labels(&v);
    if l.windows(2).any(|w| w[0] == w[1]) {
        // tick_labels finds the decimals in f32: ticks closer than that need more
        let d = (0..=12)
            .find(|&d| {
                let f: Vec<String> = v.iter().map(|x| format!("{x:.d$}")).collect();
                f.windows(2).all(|w| w[0] != w[1])
            })
            .unwrap_or(12);
        l = v
            .iter()
            .map(|x| format!("{x:.d$}").replace('-', "\u{2212}"))
            .collect();
    }
    (v, l)
}

#[cfg(test)]
mod tests {
    #[test]
    fn labels_are_distinct() {
        let (v, l) = super::labelled(0.02, 0.02 + 2e-9, 2);
        assert!(v.len() >= 2);
        assert!(l.windows(2).all(|w| w[0] != w[1]), "{l:?}");
        let (_, l) = super::labelled(0.0, 100.0, 3);
        assert_eq!(l, ["0", "50", "100"]);
    }
}
