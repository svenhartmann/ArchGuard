//! Screens, Domain and Access read one flow per document. Its own module: a test in either tab
//! would close an import cycle between them.

use crate::tab_support::Flow;

#[test]
fn screens_domain_and_access_share_one_flow_analysis() {
    let dir = tempfile::tempdir().unwrap();
    let flow = Flow::unshared(dir.path());
    let runs = std::cell::Cell::new(0);
    flow.get_with(|_| {
        runs.set(runs.get() + 1);
        Err("seeded".into())
    });
    let screens = crate::screens_ui::load(dir.path().into(), &flow);
    assert!(
        matches!(&*screens.flow, Err(e) if e == "seeded"),
        "Screens reads the shared slot"
    );
    let domain = crate::domain_ui::load(dir.path().into(), &flow);
    assert!(
        matches!(&*domain.flow, Err(e) if e == "seeded"),
        "Domain reads the shared slot"
    );
    assert!(
        matches!(&*flow.get(), Err(e) if e == "seeded"),
        "Access reads the shared slot"
    );
    assert_eq!(runs.get(), 1);
}
