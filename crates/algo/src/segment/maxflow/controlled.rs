//! Same hybrid graph-cut solver with private fallible/prepaid storage and inner polls.
use super::{CANCEL_EVERY, EPS, Graph, INF_DIST, NONE, ORPHAN, TERMINAL};
use photocraft_raster::{Interrupt, RasterInputError as E};
use std::collections::VecDeque;
fn deque<T>(n: usize) -> Option<VecDeque<T>> {
    let mut q = VecDeque::new();
    q.try_reserve_exact(n).ok()?;
    if q.capacity() > n.saturating_add(16) {
        return None;
    }
    Some(q)
}
fn push<T>(q: &mut VecDeque<T>, v: T, side: &str) -> Option<()> {
    if q.len() == q.capacity() {
        return None;
    }
    if side == "push_front" {
        q.push_front(v)
    } else {
        q.push_back(v)
    }
    Some(())
}
fn filled<T: Clone>(n: usize, value: T, cancel: &dyn Fn() -> bool) -> Option<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).ok()?;
    if v.capacity() > n.saturating_add(16) {
        return None;
    }
    while v.len() < n {
        if cancel() {
            return None;
        }
        v.resize(v.len().saturating_add(1024).min(n), value.clone());
    }
    Some(v)
}
impl Graph {
    pub(crate) fn try_with_capacity(nodes: usize, edges: usize, control: &Interrupt<'_>) -> Result<Self, E> {
        control.check().map_err(|_| E::Cancelled)?;
        if nodes >= u32::MAX as usize / 2 || edges.checked_mul(2).is_none_or(|v| v >= u32::MAX as usize - 2) {
            return Err(E::LimitExceeded);
        }
        fn prepared<T: Clone>(n: usize, value: T, control: &Interrupt<'_>) -> Result<Vec<T>, E> {
            control.check().map_err(|_| E::Cancelled)?;
            let mut out = Vec::new();
            out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
            if out.capacity() > n.saturating_add(16) {
                return Err(E::AllocationFailed);
            }
            while out.len() < n {
                control.check().map_err(|_| E::Cancelled)?;
                out.resize(out.len().saturating_add(1024).min(n), value.clone());
            }
            Ok(out)
        }
        let make = |value: u32| prepared(nodes, value, control);
        let arcs = edges.checked_mul(2).ok_or(E::LimitExceeded)?;
        let reserve = |n| {
            let mut v = Vec::new();
            v.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
            if v.capacity() > n.saturating_add(16) {
                return Err(E::AllocationFailed);
            }
            Ok(v)
        };
        Ok(Self {
            first: make(NONE)?,
            tr_cap: prepared(nodes, 0f32, control)?,
            parent: make(NONE)?,
            is_sink: prepared(nodes, false, control)?,
            ts: make(0)?,
            dist: make(0)?,
            active: prepared(nodes, false, control)?,
            head: reserve(arcs)?,
            next: reserve(arcs)?,
            r_cap: {
                let mut v = Vec::new();
                v.try_reserve_exact(arcs).map_err(|_| E::AllocationFailed)?;
                if v.capacity() > arcs.saturating_add(16) {
                    return Err(E::AllocationFailed);
                }
                v
            },
            flow: 0.,
        })
    }
    pub(crate) fn maxflow_controlled(&mut self, cancel: &dyn Fn() -> bool) -> Option<f64> {
        if cancel() {
            return None;
        }
        if self.node_count() >= 4096 {
            let mut source = false;
            let mut sink = false;
            for &capacity in &self.tr_cap {
                if cancel() {
                    return None;
                }
                if !capacity.is_finite() {
                    return self.controlled_bk(cancel);
                }
                source |= capacity > EPS;
                sink |= capacity < -EPS;
            }
            if source && sink && self.r_cap.iter().all(|v| !cancel() && v.is_finite() && *v >= 0.0) {
                return self.controlled_push_relabel(cancel);
            }
        }
        if cancel() {
            return None;
        }
        self.controlled_bk(cancel)
    }

    fn controlled_bk(&mut self, cancel: &dyn Fn() -> bool) -> Option<f64> {
        let n = self.node_count();
        let mut queue: VecDeque<u32> = deque(self.node_count().checked_mul(2)?)?;
        let mut orphans: VecDeque<u32> = deque(self.node_count().checked_mul(2)?)?;
        for i in 0..n {
            if cancel() {
                return None;
            }
            self.active[i] = false;
            self.ts[i] = 0;
            if self.tr_cap[i].abs() > EPS {
                self.is_sink[i] = self.tr_cap[i] < 0.0;
                self.parent[i] = TERMINAL;
                self.dist[i] = 1;
                self.active[i] = true;
                push(&mut queue, i as u32, "push_back")?;
            } else {
                self.parent[i] = NONE;
            }
        }
        let mut time = 0u32;
        let mut steps = 0u32;
        while let Some(i) = queue.pop_front() {
            if cancel() {
                return None;
            }
            steps = steps.wrapping_add(1);
            if steps.is_multiple_of(CANCEL_EVERY) && cancel() {
                return None;
            }
            let i = i as usize;
            self.active[i] = false;
            if self.parent[i] == NONE {
                continue;
            }
            // Growth stage.
            let mut contact = NONE;
            let sink = self.is_sink[i];
            let mut a = self.first[i];
            while a != NONE {
                if cancel() {
                    return None;
                }
                let au = a as usize;
                let j = self.head[au] as usize;
                let cap = if sink { self.r_cap[au ^ 1] } else { self.r_cap[au] };
                if cap > EPS {
                    if self.parent[j] == NONE {
                        self.is_sink[j] = sink;
                        self.parent[j] = a ^ 1;
                        self.ts[j] = self.ts[i];
                        self.dist[j] = self.dist[i] + 1;
                        if !self.active[j] {
                            self.active[j] = true;
                            push(&mut queue, j as u32, "push_back")?;
                        }
                    } else if self.is_sink[j] != sink {
                        // Arc from the S side to the T side.
                        contact = if sink { a ^ 1 } else { a };
                        break;
                    } else if self.ts[j] <= self.ts[i] && self.dist[j] > self.dist[i] {
                        // Heuristic: shorten paths to the terminal.
                        self.parent[j] = a ^ 1;
                        self.ts[j] = self.ts[i];
                        self.dist[j] = self.dist[i] + 1;
                    }
                }
                a = self.next[au];
            }
            if contact == NONE {
                continue;
            }
            // Keep scanning this node after the augmentation.
            if !self.active[i] {
                self.active[i] = true;
                push(&mut queue, i as u32, "push_front")?;
            }
            time = time.checked_add(1)?;
            self.controlled_augment(contact as usize, &mut orphans, cancel)?;
            // Adoption stage.
            while let Some(o) = orphans.pop_front() {
                if cancel() {
                    return None;
                }
                self.controlled_adopt(o as usize, time, &mut queue, &mut orphans, cancel)?;
            }
        }
        Some(self.flow)
    }

    fn controlled_augment(&mut self, m: usize, orphans: &mut VecDeque<u32>, cancel: &dyn Fn() -> bool) -> Option<()> {
        let u = self.head[m ^ 1] as usize;
        let v = self.head[m] as usize;
        // Bottleneck.
        let mut b = self.r_cap[m];
        let mut x = u;
        loop {
            if cancel() {
                return None;
            }
            let a = self.parent[x];
            if a == TERMINAL {
                break;
            }
            b = b.min(self.r_cap[a as usize ^ 1]);
            x = self.head[a as usize] as usize;
        }
        b = b.min(self.tr_cap[x]);
        let mut x = v;
        loop {
            if cancel() {
                return None;
            }
            let a = self.parent[x];
            if a == TERMINAL {
                break;
            }
            b = b.min(self.r_cap[a as usize]);
            x = self.head[a as usize] as usize;
        }
        b = b.min(-self.tr_cap[x]);
        // Push.
        self.r_cap[m ^ 1] += b;
        self.r_cap[m] -= b;
        let mut x = u;
        loop {
            if cancel() {
                return None;
            }
            let a = self.parent[x];
            if a == TERMINAL {
                break;
            }
            let au = a as usize;
            self.r_cap[au] += b;
            self.r_cap[au ^ 1] -= b;
            let up = self.head[au] as usize;
            if self.r_cap[au ^ 1] <= EPS {
                self.parent[x] = ORPHAN;
                push(orphans, x as u32, "push_back")?;
            }
            x = up;
        }
        self.tr_cap[x] -= b;
        if self.tr_cap[x] <= EPS {
            self.parent[x] = ORPHAN;
            push(orphans, x as u32, "push_back")?;
        }
        let mut x = v;
        loop {
            if cancel() {
                return None;
            }
            let a = self.parent[x];
            if a == TERMINAL {
                break;
            }
            let au = a as usize;
            self.r_cap[au ^ 1] += b;
            self.r_cap[au] -= b;
            let up = self.head[au] as usize;
            if self.r_cap[au] <= EPS {
                self.parent[x] = ORPHAN;
                push(orphans, x as u32, "push_back")?;
            }
            x = up;
        }
        self.tr_cap[x] += b;
        if self.tr_cap[x] >= -EPS {
            self.parent[x] = ORPHAN;
            push(orphans, x as u32, "push_back")?;
        }
        self.flow += b as f64;
        Some(())
    }

    fn controlled_adopt(&mut self, i: usize, time: u32, queue: &mut VecDeque<u32>, orphans: &mut VecDeque<u32>, cancel: &dyn Fn() -> bool) -> Option<()> {
        let sink = self.is_sink[i];
        let mut best = NONE;
        let mut d_min = INF_DIST;
        let mut a = self.first[i];
        while a != NONE {
            if cancel() {
                return None;
            }
            let au = a as usize;
            let cap = if sink { self.r_cap[au] } else { self.r_cap[au ^ 1] };
            let j = self.head[au] as usize;
            if cap > EPS && self.is_sink[j] == sink && self.parent[j] != NONE {
                // Does j's path lead to a terminal?
                let mut d = 0u32;
                let mut k = j;
                loop {
                    if cancel() {
                        return None;
                    }
                    if self.ts[k] == time {
                        d = d.saturating_add(self.dist[k]);
                        break;
                    }
                    let pa = self.parent[k];
                    d += 1;
                    if pa == TERMINAL {
                        self.ts[k] = time;
                        self.dist[k] = 1;
                        break;
                    }
                    if pa == ORPHAN {
                        d = INF_DIST;
                        break;
                    }
                    k = self.head[pa as usize] as usize;
                }
                if d < INF_DIST {
                    if d < d_min {
                        best = a;
                        d_min = d;
                    }
                    // Cache distances along the path.
                    let mut k = j;
                    let mut dd = d;
                    while self.ts[k] != time {
                        if cancel() {
                            return None;
                        }
                        self.ts[k] = time;
                        self.dist[k] = dd;
                        dd -= 1;
                        k = self.head[self.parent[k] as usize] as usize;
                    }
                }
            }
            a = self.next[au];
        }
        if best != NONE {
            self.parent[i] = best;
            self.ts[i] = time;
            self.dist[i] = d_min + 1;
            return Some(());
        }
        // No parent: i becomes free.
        self.parent[i] = NONE;
        let mut a = self.first[i];
        while a != NONE {
            if cancel() {
                return None;
            }
            let au = a as usize;
            let j = self.head[au] as usize;
            let pj = self.parent[j];
            if self.is_sink[j] == sink && pj != NONE {
                let cap = if sink { self.r_cap[au] } else { self.r_cap[au ^ 1] };
                if cap > EPS && !self.active[j] {
                    self.active[j] = true;
                    push(queue, j as u32, "push_back")?;
                }
                if pj != TERMINAL && pj != ORPHAN && self.head[pj as usize] as usize == i {
                    self.parent[j] = ORPHAN;
                    push(orphans, j as u32, "push_back")?;
                }
            }
            a = self.next[au];
        }
        Some(())
    }
    pub(super) fn controlled_push_relabel(&mut self, cancel: &dyn Fn() -> bool) -> Option<f64> {
        let n = self.node_count();
        let Some(source_height) = u32::try_from(n).ok().filter(|&n| n < u32::MAX / 2).map(|n| n + 2) else {
            return self.controlled_bk(cancel);
        };
        if cancel() {
            return None;
        }
        let mut back = filled(n, 0.0f32, cancel)?;
        let mut excess = filled(n, 0.0f64, cancel)?;
        for i in 0..n {
            if cancel() {
                return None;
            }
            self.active[i] = false;
            if self.tr_cap[i] > EPS {
                back[i] = self.tr_cap[i];
                excess[i] = f64::from(self.tr_cap[i]);
                self.tr_cap[i] = 0.0;
            }
        }
        self.controlled_global_relabel(source_height, &back, cancel)?;
        let mut queue = deque(n.checked_mul(2)?)?;
        for (i, &e) in excess.iter().enumerate() {
            if cancel() {
                return None;
            }
            if e > f64::from(EPS) {
                self.active[i] = true;
                push(&mut queue, i, "push_back")?
            }
        }
        let mut work = 0usize;
        let frequency = self.r_cap.len().saturating_mul(2).saturating_add(n);
        let mut steps = 0u32;
        while let Some(i) = queue.pop_front() {
            if cancel() {
                return None;
            }
            steps = steps.wrapping_add(1);
            if steps.is_multiple_of(CANCEL_EVERY) && cancel() {
                return None;
            }
            self.active[i] = false;
            while excess[i] > f64::from(EPS) {
                if cancel() {
                    return None;
                }
                if self.dist[i] == 1 && self.tr_cap[i] < -EPS {
                    let delta = (excess[i] as f32).min(-self.tr_cap[i]);
                    self.tr_cap[i] += delta;
                    excess[i] -= f64::from(delta);
                    self.flow += f64::from(delta);
                    if excess[i] <= f64::from(EPS) {
                        break;
                    }
                }
                if self.dist[i] == source_height + 1 && back[i] > EPS {
                    let delta = (excess[i] as f32).min(back[i]);
                    back[i] -= delta;
                    self.tr_cap[i] += delta;
                    excess[i] -= f64::from(delta);
                    if excess[i] <= f64::from(EPS) {
                        break;
                    }
                }
                let mut a = self.parent[i];
                while a != NONE {
                    if cancel() {
                        return None;
                    }
                    let ai = a as usize;
                    let j = self.head[ai] as usize;
                    work += 1;
                    // `checked_add`: two nodes a global relabel left unreachable (`u32::MAX`) must
                    // not count as admissible for each other, or float-stranded excess ping-pongs
                    // between them forever.
                    if self.r_cap[ai] > EPS && self.dist[j].checked_add(1) == Some(self.dist[i]) {
                        let delta = (excess[i] as f32).min(self.r_cap[ai]);
                        self.r_cap[ai] -= delta;
                        self.r_cap[ai ^ 1] += delta;
                        excess[i] -= f64::from(delta);
                        excess[j] += f64::from(delta);
                        if !self.active[j] && excess[j] > f64::from(EPS) {
                            self.active[j] = true;
                            push(&mut queue, j, "push_back")?;
                        }
                        if excess[i] <= f64::from(EPS) {
                            break;
                        }
                    }
                    a = self.next[ai];
                    self.parent[i] = a;
                }
                if excess[i] <= f64::from(EPS) {
                    break;
                }
                let mut height = if self.tr_cap[i] < -EPS { 0 } else { u32::MAX };
                if back[i] > EPS {
                    height = height.min(source_height)
                }
                let mut a = self.first[i];
                while a != NONE {
                    if cancel() {
                        return None;
                    }
                    let ai = a as usize;
                    work += 1;
                    if self.r_cap[ai] > EPS {
                        height = height.min(self.dist[self.head[ai] as usize])
                    }
                    a = self.next[ai];
                }
                if height == u32::MAX {
                    break;
                }
                self.dist[i] = height.saturating_add(1);
                self.parent[i] = self.first[i];
            }
            if work >= frequency {
                self.controlled_global_relabel(source_height, &back, cancel)?;
                work = 0;
            }
        }
        // Report the smallest source-side cut, independently of discharge order.
        for i in 0..n {
            if cancel() {
                return None;
            }
            self.parent[i] = NONE;
            self.is_sink[i] = true;
        }
        for i in 0..n {
            if cancel() {
                return None;
            }
            if self.tr_cap[i] > EPS {
                self.parent[i] = TERMINAL;
                self.is_sink[i] = false;
                push(&mut queue, i, "push_back")?;
            }
        }
        while let Some(i) = queue.pop_front() {
            if cancel() {
                return None;
            }
            let mut a = self.first[i];
            while a != NONE {
                if cancel() {
                    return None;
                }
                let ai = a as usize;
                let j = self.head[ai] as usize;
                if self.r_cap[ai] > EPS && self.parent[j] == NONE {
                    self.parent[j] = TERMINAL;
                    self.is_sink[j] = false;
                    push(&mut queue, j, "push_back")?;
                }
                a = self.next[ai];
            }
        }
        Some(self.flow)
    }

    fn controlled_global_relabel(&mut self, source_height: u32, back: &[f32], cancel: &dyn Fn() -> bool) -> Option<()> {
        let n = self.node_count();
        for i in 0..n {
            if cancel() {
                return None;
            }
            self.dist[i] = u32::MAX;
            self.parent[i] = self.first[i];
        }
        let mut queue = deque(n.checked_mul(2)?)?;
        for i in 0..self.node_count() {
            if cancel() {
                return None;
            }
            if self.tr_cap[i] < -EPS {
                self.dist[i] = 1;
                push(&mut queue, i, "push_back")?
            }
        }
        while let Some(i) = queue.pop_front() {
            if cancel() {
                return None;
            }
            let mut a = self.first[i];
            while a != NONE {
                if cancel() {
                    return None;
                }
                let ai = a as usize;
                let j = self.head[ai] as usize;
                if self.r_cap[ai ^ 1] > EPS && self.dist[j] == u32::MAX {
                    self.dist[j] = self.dist[i].saturating_add(1);
                    push(&mut queue, j, "push_back")?;
                }
                a = self.next[ai];
            }
        }
        // Give sink-unreachable vertices exact distances back to the source.
        // Resetting all of them to the same height can undo discharge progress.
        for (i, &capacity) in back.iter().enumerate() {
            if cancel() {
                return None;
            }
            if capacity > EPS && self.dist[i] == u32::MAX {
                self.dist[i] = source_height + 1;
                push(&mut queue, i, "push_back")?;
            }
        }
        while let Some(i) = queue.pop_front() {
            if cancel() {
                return None;
            }
            let mut a = self.first[i];
            while a != NONE {
                if cancel() {
                    return None;
                }
                let ai = a as usize;
                let j = self.head[ai] as usize;
                if self.r_cap[ai ^ 1] > EPS && self.dist[j] == u32::MAX {
                    self.dist[j] = self.dist[i].saturating_add(1);
                    push(&mut queue, j, "push_back")?;
                }
                a = self.next[ai];
            }
        }
        Some(())
    }
}
