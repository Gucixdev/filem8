//! Tiny fzf-style subsequence scorer. Both inputs should be lowercased.

/// `Some(score)` (higher is better) when every char of `q` appears in order in `s`.
pub fn score(q: &str, s: &str) -> Option<i32> {
    if q.is_empty() {
        return Some(0);
    }
    let sc: Vec<char> = s.chars().collect();
    let mut si = 0;
    let mut score = 0;
    let mut prev: Option<usize> = None;
    for qc in q.chars() {
        let mut found = None;
        while si < sc.len() {
            if sc[si] == qc {
                found = Some(si);
                si += 1;
                break;
            }
            si += 1;
        }
        let i = found?;
        score += 10;
        if i == 0 { score += 15; }
        if i > 0 && matches!(sc[i - 1], '/' | '_' | '-' | ' ' | '.') { score += 8; }
        if let Some(p) = prev { if p + 1 == i { score += 12; } else { score -= (i - p - 1).min(8) as i32; } }
        prev = Some(i);
    }
    score -= (sc.len() as i32 - q.chars().count() as i32).min(20) / 4;
    Some(score)
}

/// Rank `items` by fuzzy score for `q`, best first, keep `limit`.
pub fn rank<'a>(q: &str, items: impl Iterator<Item = &'a str>, limit: usize) -> Vec<(i32, &'a str)> {
    let q = q.to_lowercase();
    let mut v: Vec<(i32, &str)> = items.filter_map(|s| score(&q, &s.to_lowercase()).map(|sc| (sc, s))).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
    v.truncate(limit);
    v
}

/// Recursive fuzzy file search below `root` (depth and count bounded, skips
/// hidden dirs and common junk).
pub fn walk(root: &std::path::Path, q: &str, max_files: usize, max_depth: usize) -> Vec<std::path::PathBuf> {
    let mut files = vec![];
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for de in rd.flatten() {
            let name = de.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" || name == "target" { continue; }
            let p = de.path();
            let is_dir = de.file_type().map(|t| t.is_dir()).unwrap_or(false);
            files.push(p.clone());
            if is_dir && depth < max_depth { stack.push((p, depth + 1)); }
            if files.len() >= max_files { break; }
        }
        if files.len() >= max_files { break; }
    }
    let rels: Vec<String> = files.iter().map(|p| p.strip_prefix(root).unwrap_or(p).to_string_lossy().into_owned()).collect();
    let ranked = rank(q, rels.iter().map(|s| s.as_str()), 200);
    ranked.into_iter().map(|(_, r)| root.join(r)).collect()
}
