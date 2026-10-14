//! Controlled bounded unit-RGB fit companion; legacy public fitting is unchanged.
use super::{Acc, Gmm, Rng, nearest};
use photocraft_raster::{Interrupt, RasterInputError as E};
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn filled<T: Clone>(n: usize, value: T, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() > n.saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    while out.len() < n {
        poll(c)?;
        out.resize(out.len().saturating_add(256).min(n), value.clone());
    }
    Ok(out)
}
fn reserved<T>(n: usize) -> Result<Vec<T>, E> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() > n.saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    Ok(out)
}
fn centers(samples: &[[f32; 3]], k: usize, c: &Interrupt<'_>) -> Result<Vec<[f32; 3]>, E> {
    let mut rng = Rng::new(0x5EED + samples.len() as u64);
    let at = (rng.next_u64() % samples.len() as u64) as usize;
    let first = *samples.get(at).ok_or(E::LengthMismatch)?;
    let mut centers = reserved(k)?;
    centers.push(first);
    let mut dist = filled(samples.len(), 0f32, c)?;
    for (i, (d, z)) in dist.iter_mut().zip(samples).enumerate() {
        if i.is_multiple_of(64) {
            poll(c)?;
        }
        *d = super::super::d2(*z, first);
    }
    while centers.len() < k {
        poll(c)?;
        let mut total = 0f64;
        for (i, d) in dist.iter().enumerate() {
            if i.is_multiple_of(64) {
                poll(c)?;
            }
            total += f64::from(*d);
        }
        if total <= 1e-12 {
            break;
        }
        let mut t = f64::from(rng.f32()) * total;
        let mut pick = samples.len() - 1;
        for (i, d) in dist.iter().enumerate() {
            if i.is_multiple_of(64) {
                poll(c)?;
            }
            t -= f64::from(*d);
            if t <= 0. {
                pick = i;
                break;
            }
        }
        let selected = *samples.get(pick).ok_or(E::LengthMismatch)?;
        centers.push(selected);
        for (i, (d, z)) in dist.iter_mut().zip(samples).enumerate() {
            if i.is_multiple_of(64) {
                poll(c)?;
            }
            *d = d.min(super::super::d2(*z, selected));
        }
    }
    for _ in 0..10 {
        poll(c)?;
        let mut sums = filled(centers.len(), ([0f64; 3], 0usize), c)?;
        for (i, z) in samples.iter().enumerate() {
            if i.is_multiple_of(64) {
                poll(c)?;
            }
            let at = nearest(&centers, *z);
            let sum = sums.get_mut(at).ok_or(E::LengthMismatch)?;
            for (acc, value) in sum.0.iter_mut().zip(z) {
                *acc += f64::from(*value);
            }
            sum.1 += 1;
        }
        let mut next = reserved(k)?;
        for (sum, n) in &sums {
            if *n > 0 {
                next.push(sum.map(|v| (v / *n as f64) as f32));
            }
        }
        let done = next == centers;
        centers = next;
        if done {
            break;
        }
    }
    poll(c)?;
    Ok(centers)
}
fn from_labels(samples: &[[f32; 3]], labels: &[usize], k: usize, reg: f32, c: &Interrupt<'_>) -> Result<Option<Gmm>, E> {
    let mut acc = filled(k, Acc::default(), c)?;
    for (i, (z, label)) in samples.iter().zip(labels).enumerate() {
        if i.is_multiple_of(64) {
            poll(c)?;
        }
        acc.get_mut(*label).ok_or(E::LengthMismatch)?.add(*z, 1.);
    }
    let mut comps = reserved(k)?;
    for a in acc {
        poll(c)?;
        if let Some(component) = a.component(samples.len() as f64, reg) {
            comps.push(component);
        }
    }
    Ok((!comps.is_empty()).then_some(Gmm { comps, reg }))
}
fn em_step(g: &Gmm, samples: &[[f32; 3]], c: &Interrupt<'_>) -> Result<Option<Gmm>, E> {
    let k = g.comps.len();
    let mut acc = filled(k, Acc::default(), c)?;
    let mut lw = filled(k, 0f64, c)?;
    for (i, z) in samples.iter().enumerate() {
        if i.is_multiple_of(64) {
            poll(c)?;
        }
        let mut mx = f64::MIN;
        for (component, l) in g.comps.iter().zip(lw.iter_mut()) {
            *l = component.log_weighted(*z);
            mx = mx.max(*l);
        }
        let mut sum = 0.;
        for l in &mut lw {
            *l = (*l - mx).exp();
            sum += *l;
        }
        for (a, l) in acc.iter_mut().zip(&lw) {
            let responsibility = l / sum;
            if responsibility > 1e-6 {
                a.add(*z, responsibility);
            }
        }
    }
    let mut comps = reserved(k)?;
    for a in acc {
        poll(c)?;
        if let Some(component) = a.component(samples.len() as f64, g.reg) {
            comps.push(component);
        }
    }
    Ok((!comps.is_empty()).then_some(Gmm { comps, reg: g.reg }))
}
impl Gmm {
    /// Same deterministic k-means++/10 Lloyd/3 EM recipe for <=4000 unit RGB
    /// samples and <=16 components, with fallible bounded scratch and inner polls.
    pub fn fit_with_interrupt(samples: &[[f32; 3]], k: usize, reg: f32, c: &Interrupt<'_>) -> Result<Option<Self>, E> {
        Self::fit_bounded(samples, k, reg, 4000, c)
    }
    /// GrabCut's unchanged forty-thousand sample recipe, separately admitted by its caller.
    pub(crate) fn fit_grabcut_with_interrupt(samples: &[[f32; 3]], k: usize, reg: f32, c: &Interrupt<'_>) -> Result<Option<Self>, E> {
        Self::fit_bounded(samples, k, reg, 40_000, c)
    }
    fn fit_bounded(samples: &[[f32; 3]], k: usize, reg: f32, max_samples: usize, c: &Interrupt<'_>) -> Result<Option<Self>, E> {
        poll(c)?;
        if samples.is_empty() {
            return Ok(None);
        }
        if samples.len() > max_samples || k > 16 || !reg.is_finite() || reg < 0. {
            return Err(E::LimitExceeded);
        }
        for (i, z) in samples.iter().enumerate() {
            if i.is_multiple_of(64) {
                poll(c)?;
            }
            if z.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return Err(E::InvalidSample);
            }
        }
        let k = k.clamp(1, samples.len());
        let means = centers(samples, k, c)?;
        let mut labels = filled(samples.len(), 0usize, c)?;
        for (i, (label, z)) in labels.iter_mut().zip(samples).enumerate() {
            if i.is_multiple_of(64) {
                poll(c)?;
            }
            *label = nearest(&means, *z);
        }
        let Some(mut g) = from_labels(samples, &labels, means.len(), reg, c)? else { return Ok(None) };
        for _ in 0..3 {
            poll(c)?;
            if let Some(next) = em_step(&g, samples, c)? {
                g = next;
            }
        }
        poll(c)?;
        Ok(Some(g))
    }
    pub(crate) fn refit_grabcut_with_interrupt(&self, samples: &[[f32; 3]], c: &Interrupt<'_>) -> Result<Option<Self>, E> {
        poll(c)?;
        if samples.len() > 40_000 || self.comps.len() > 16 {
            return Err(E::LimitExceeded);
        }
        let mut labels = reserved(samples.len())?;
        for (i, z) in samples.iter().enumerate() {
            if i.is_multiple_of(64) {
                poll(c)?;
            }
            if z.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return Err(E::InvalidSample);
            }
            labels.push(self.component_of(*z));
        }
        from_labels(samples, &labels, self.comps.len(), self.reg, c)
    }
    pub(crate) fn clone_grabcut_with_interrupt(&self, c: &Interrupt<'_>) -> Result<Self, E> {
        if self.comps.len() > 16 {
            return Err(E::LimitExceeded);
        }
        let mut comps = reserved(self.comps.len())?;
        for component in &self.comps {
            poll(c)?;
            comps.push(component.clone());
        }
        Ok(Self { comps, reg: self.reg })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_fit_matches_baseline_seed_and_steps() {
        let samples = (0..97).map(|i| [(i % 11) as f32 / 11., (i % 7) as f32 / 7., (i % 13) as f32 / 13.]).collect::<Vec<_>>();
        assert_eq!(Gmm::fit_with_interrupt(&samples, 3, 1e-4, &Interrupt::NONE).unwrap(), Gmm::fit(&samples, 3, 1e-4));
        assert_eq!(Gmm::fit_with_interrupt(&samples, 3, 1e-4, &Interrupt::cancel_only(&|| true)), Err(E::Cancelled));
    }
}
