//! The Named People cards follow the grid's photos: choosing an album in the sidebar keeps the People view open
//! and narrows the cards to the people named in that album's photos, with the counts and view title of what is shown.

use std::time::Duration;

use lightcraft_catalog::{Op, PhotoId};
use serde_json::{Value, json};

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn face(x: f64, name: &str) -> lightcraft_meta::Region {
    lightcraft_meta::Region {
        rect: lightcraft_geom::Rect { x0: x, y0: 0.2, x1: x + 0.2, y1: 0.6 },
        kind: lightcraft_meta::RegionKind::Face,
        name: Some(name.to_string()),
        description: None,
        rejected: Vec::new(),
    }
}

fn set(h: &mut Headless, id: PhotoId, regions: Vec<lightcraft_meta::Region>) {
    let mut meta = h.app.session.catalog.photo(id).unwrap().meta.clone();
    meta.regions = regions;
    h.app.session.commit("setup", Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
}

fn run(h: &mut Headless, command: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    h.step();
    h.step();
    h.settle(T);
    r["result"].clone()
}

fn cards(h: &mut Headless) -> Vec<String> {
    let r = h.request("ui.widgets", json!({"filter": "person:"}), T);
    let mut v: Vec<String> = r["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w["id"].as_str())
        .filter_map(|id| id.strip_prefix("person:").map(str::to_string))
        .collect();
    v.sort();
    v
}

#[test]
fn choosing_an_album_keeps_the_people_view_and_narrows_the_cards() {
    let services = Services { png: None, write_shared: Some(std::sync::Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let mut h = Headless::new(LightcraftApp::new(lightcraft_engine::Session::with_demo(), services), [1400.0, 900.0], 1.0);
    h.settle(T);
    let ids: Vec<_> = h.app.session.catalog.photos().map(|p| p.id).take(2).collect();
    set(&mut h, ids[0], vec![face(0.1, "Ann")]);
    set(&mut h, ids[1], vec![face(0.3, "Bob")]);
    run(&mut h, "view.people", json!({}));
    assert_eq!(cards(&mut h), ["Ann", "Bob"]);

    let album = run(&mut h, "album.create", json!({"name": "Ann's"}))["id"].as_u64().unwrap();
    run(&mut h, "album.addPhotos", json!({"id": album, "ids": [ids[0].0]}));
    run(&mut h, "library.source", json!({"kind": "album", "id": album}));
    assert_eq!(h.app.ui.view, crate::state::ViewMode::People, "the People view stays open");
    assert_eq!(cards(&mut h), ["Ann"], "only the people named in the album");
    assert_eq!(run(&mut h, "view.people", json!({}))["people"], 1);

    run(&mut h, "library.source", json!({"kind": "all"}));
    assert_eq!(cards(&mut h), ["Ann", "Bob"]);
}
