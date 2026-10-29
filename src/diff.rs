#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Same,
    Add,
    Del,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: Kind,
    pub text: String,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
}

/// Line diff: trims the common prefix and suffix, then runs LCS on what is left (or a plain swap when it is huge).
pub fn diff_lines(before: &str, after: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = if before.is_empty() { vec![] } else { before.split('\n').collect() };
    let b: Vec<&str> = if after.is_empty() { vec![] } else { after.split('\n').collect() };
    let mut start = 0;
    while start < a.len() && start < b.len() && a[start] == b[start] {
        start += 1;
    }
    let (mut end_a, mut end_b) = (a.len(), b.len());
    while end_a > start && end_b > start && a[end_a - 1] == b[end_b - 1] {
        end_a -= 1;
        end_b -= 1;
    }
    let (ma, mb) = (&a[start..end_a], &b[start..end_b]);
    let mut mid: Vec<(Kind, &str)> = vec![];
    if ma.len() * mb.len() > 4_000_000 {
        mid.extend(ma.iter().map(|t| (Kind::Del, *t)));
        mid.extend(mb.iter().map(|t| (Kind::Add, *t)));
    } else {
        let (n, m) = (ma.len(), mb.len());
        let w = m + 1;
        let mut dp = vec![0u32; (n + 1) * w];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                dp[i * w + j] = if ma[i] == mb[j] { dp[(i + 1) * w + j + 1] + 1 } else { dp[(i + 1) * w + j].max(dp[i * w + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && ma[i] == mb[j] {
                mid.push((Kind::Same, ma[i]));
                i += 1;
                j += 1;
            } else if i < n && (j >= m || dp[(i + 1) * w + j] >= dp[i * w + j + 1]) {
                // Removals first, so a changed line reads old then new.
                mid.push((Kind::Del, ma[i]));
                i += 1;
            } else {
                mid.push((Kind::Add, mb[j]));
                j += 1;
            }
        }
    }
    let line = |kind, text: &str, o: Option<usize>, n: Option<usize>| DiffLine { kind, text: text.to_string(), old_no: o, new_no: n };
    let mut out: Vec<DiffLine> = (0..start).map(|i| line(Kind::Same, a[i], Some(i + 1), Some(i + 1))).collect();
    let (mut o, mut nn) = (start, start);
    for (k, t) in mid {
        match k {
            Kind::Same => {
                o += 1;
                nn += 1;
                out.push(line(k, t, Some(o), Some(nn)));
            }
            Kind::Del => {
                o += 1;
                out.push(line(k, t, Some(o), None));
            }
            Kind::Add => {
                nn += 1;
                out.push(line(k, t, None, Some(nn)));
            }
        }
    }
    for t in &a[end_a..] {
        o += 1;
        nn += 1;
        out.push(line(Kind::Same, t, Some(o), Some(nn)));
    }
    out
}

/// Keeps changed lines plus `context` lines around them; gaps become None.
pub fn hunks(lines: &[DiffLine], context: usize) -> Vec<Option<&DiffLine>> {
    let mut keep = vec![false; lines.len()];
    for (i, l) in lines.iter().enumerate() {
        if l.kind != Kind::Same {
            for k in i.saturating_sub(context)..=(i + context).min(lines.len().saturating_sub(1)) {
                keep[k] = true;
            }
        }
    }
    let mut out = vec![];
    let mut gap = false;
    for (i, l) in lines.iter().enumerate() {
        if keep[i] {
            if gap && !out.is_empty() {
                out.push(None);
            }
            gap = false;
            out.push(Some(l));
        } else {
            gap = true;
        }
    }
    out
}

pub fn stats(lines: &[DiffLine]) -> (usize, usize) {
    let added = lines.iter().filter(|l| l.kind == Kind::Add).count();
    let removed = lines.iter().filter(|l| l.kind == Kind::Del).count();
    (added, removed)
}
