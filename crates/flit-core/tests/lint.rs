//! Time is information, never order: no ordering, conflict, sync or fold
//! code reads a time field, and no clock is read outside `clock.rs`.

fn src(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn ordering_code_never_reads_time() {
    let is_code = |l: &&str| !l.trim_start().starts_with("//");
    // the fold, the verify tier, the keeper's decisions and the state model
    // never see a time field at all
    for f in ["seal/fold.rs", "seal/verify.rs", "state.rs", "crew.rs", "beings.rs"] {
        let s = src(f);
        for word in ["stamped_at", "received_at", "wall_ms", "now_ms", "SystemTime", "Instant"] {
            let hits: Vec<&str> = s.lines().filter(is_code).filter(|l| l.contains(word)).collect();
            assert!(hits.is_empty(), "{f} reads {word}: {hits:?}");
        }
    }
    // the keeper only passes now_ms through to the History act it stamps
    let k = src("seal/keeper.rs");
    for word in ["stamped_at", "received_at", "wall_ms", "SystemTime", "Instant"] {
        let hits: Vec<&str> = k.lines().filter(is_code).filter(|l| l.contains(word) && !l.contains("stamped_at: now_ms")).collect();
        assert!(hits.is_empty(), "keeper reads {word}: {hits:?}");
    }
    let reads: Vec<&str> = k.lines().filter(is_code).filter(|l| l.contains("now_ms") && !l.contains("stamped_at: now_ms") && !l.contains("now_ms: u64") && !l.contains("now_ms)")).collect();
    assert!(reads.is_empty(), "{reads:?}");
    // the chains store received_at for display (the sidecar) and never
    // compare, sort or branch on it
    let m = src("seal/mod.rs");
    assert!(m.lines().filter(is_code).all(|l| !l.contains("stamped_at")));
    for l in m.lines().filter(is_code).filter(|l| l.contains("received_at")) {
        for bad in ["sort", "cmp", "min(", "max(", " < ", " > ", " <= ", " >= ", " == "] {
            assert!(!l.contains(bad), "chains decide by received_at: {l}");
        }
    }
    // lenses may carry the witness times but never sort by them
    let l = src("seal/lens.rs");
    for line in l.lines().filter(is_code) {
        if line.contains("sort") {
            assert!(!line.contains("stamped_at") && !line.contains("received_at"), "lens sorts by time: {line}");
        }
    }
}

#[test]
fn only_the_clock_module_reads_a_clock() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&root, &mut files);
    for p in files {
        if p.ends_with("clock.rs") {
            continue;
        }
        let s = std::fs::read_to_string(&p).unwrap();
        for word in ["SystemTime", "Instant::now", "time::now"] {
            assert!(!s.contains(word), "{} reads a clock ({word})", p.display());
        }
    }
}

#[test]
fn seal_module_has_a_small_interface() {
    // the rest of the crate talks to Chains, Song and the seal submodules
    // only through their public items; no other crate reaches inside
    let s = src("seal/mod.rs");
    assert!(s.contains("pub(crate) acts"), "fields stay crate-private so the module can be swapped");
}
