//! Public Dissolve coverage endpoints; partial-coverage randomness stays closed.
//! No application-specific seed or pixel pattern is invented or substituted.
use super::{CompositionError, Result};
pub(crate) fn endpoint(back: [f64; 4], source: [f64; 4], opacity: f64) -> Result<[f64; 4]> {
    if back.iter().chain(source.iter()).chain(std::iter::once(&opacity)).any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(CompositionError::Unsupported("Dissolve finite unit RGB/alpha domain is not qualified"));
    }
    if source[3] == 0.0 || opacity == 0.0 {
        return Ok(back);
    }
    if source[3] == 1.0 && opacity == 1.0 {
        return Ok([source[0], source[1], source[2], 1.0]);
    }
    Err(CompositionError::Unsupported("GIMP Dissolve partial-coverage random pattern is not qualified"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_are_exact_without_a_stochastic_substitute() {
        let marker = 0.5 + 2.0_f64.powi(-40);
        let back = [marker, -0.0, 0.125, 0.25];
        let source = [0.2, marker, -0.0, 1.0];
        let transparent = endpoint(back, source, 0.0).unwrap();
        let opaque = endpoint(back, source, 1.0).unwrap();
        for c in 0..4 {
            assert_eq!(transparent[c].to_bits(), back[c].to_bits());
            assert_eq!(opaque[c].to_bits(), source[c].to_bits());
        }
        assert!(endpoint(back, source, 0.5).is_err());
        assert!(endpoint(back, [0.2, marker, 0.0, 0.5], 1.0).is_err());
        assert!(endpoint(back, [0.2, marker, 0.0, 1e-300], 1e-300).is_err());
        assert!(endpoint(back, [f64::NAN, 0.0, 0.0, 0.0], 0.0).is_err());
        assert!(endpoint(back, source, f64::INFINITY).is_err());
        assert!(endpoint(back, [-1.0, 0.0, 0.0, 0.0], 0.0).is_err());
    }
    #[test]
    #[ignore = "24MP release Dissolve endpoint/error throughput measurement"]
    fn dissolve_endpoints_24mp_release() {
        for opacity in [0.0, 1.0, 0.5] {
            let start = std::time::Instant::now();
            for i in 0..24_000_000 {
                let b = std::hint::black_box([0.15 + f64::from(i % 128) * 0.001, 0.6, 0.85, 0.7]);
                let result = endpoint(b, [0.8, 0.25, 0.4, 1.0], opacity);
                let _ = std::hint::black_box(result);
            }
            println!("Dissolve endpoint opacity{opacity}: 24MP {:?}", start.elapsed());
        }
    }
}
