use rstest::*;

use csv::ReaderBuilder;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::Instant;
use tracing::info;

use serde_json::Value;

use berlin_core::location::CsvLocode;
use berlin_core::locations_db::{parse_data_block, parse_data_list, LocationsDb};
use berlin_core::search::SearchTerm;

#[fixture]
#[once]
pub fn fake_data() -> LocationsDb {
    let start = Instant::now();
    let db = LocationsDb::default();
    let db = RwLock::new(db);

    let mut data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    data_dir.extend(["tests", "data"]);

    let code_json = data_dir.join("test-codes.json");
    let path = code_json.as_path();
    let path_str = data_dir.display();
    info!("Path {path_str:?}");
    let fo = File::open(path).expect("cannot open json file");
    let reader = BufReader::new(fo);
    let json: serde_json::Value = serde_json::from_reader(reader).expect("cannot decode json");
    info!("Decode json file {path_str}: {:.2?}", start.elapsed());
    match json {
        Value::Object(obj) => {
            parse_data_block(&db, obj).expect("cannot parse json");
        }
        other => panic!("Expected a JSON object: {:?}", other),
    }

    let mut db = db.into_inner().expect("rw lock extract");
    let csv_file = data_dir.join("test-code-list.csv");
    let csv_file_open = File::open(csv_file).expect("Read CSV File");
    let mut csv_reader = ReaderBuilder::new().from_reader(csv_file_open);
    let iter = csv_reader
        .deserialize::<CsvLocode>()
        .enumerate()
        .map(|(n, result)| result.expect(format!("could not parse CSV line {}", n + 1).as_str()));
    db = parse_data_list(db, iter).expect("could not parse csv file");
    let count = db.all.len();
    info!("parsed {} locations in: {:.2?}", count, start.elapsed());
    db.mk_fst()
}

#[fixture]
pub fn search_lyuliakovo() -> SearchTerm {
    SearchTerm::from_raw_query("Lyuliakovo".to_string(), None, 5, 3)
}

#[fixture]
pub fn search_abercorn() -> SearchTerm {
    SearchTerm::from_raw_query("abercorn".to_string(), None, 5, 3)
}

#[rstest]
fn should_load_codes(fake_data: &LocationsDb) {
    assert!(
        fake_data.all.len() == 17,
        "Got {} codes",
        fake_data.all.len()
    )
}

#[rstest]
fn should_search_lyuliakovo(fake_data: &LocationsDb, search_lyuliakovo: SearchTerm) {
    let results = fake_data.search(&search_lyuliakovo);
    assert![results.len() == 1];

    let lyuliakovo = results[0].0;
    assert![lyuliakovo == "UN-LOCODE-bg:blo"];

    let lyuliakovo_loc = &fake_data.all[&lyuliakovo];

    assert![lyuliakovo_loc.get_state() == "bg"];
    assert![lyuliakovo_loc.get_subdiv().unwrap() == "02"];
}

#[rstest]
fn should_search_abercorn(fake_data: &LocationsDb, search_abercorn: SearchTerm) {
    let results = fake_data.search(&search_abercorn);
    assert![results.len() == 1];

    let abercarn = results[0].0;
    assert![abercarn == "UN-LOCODE-gb:abc"];

    let abercarn_loc = &fake_data.all[&abercarn];

    assert![abercarn_loc.get_state() == "gb"];
    assert![abercarn_loc.get_subdiv().unwrap() == "cay"];
}

#[rstest]
fn should_search_long_sentence(fake_data: &LocationsDb) {
    pub struct LongSearch {
        pub q: &'static str,
        pub r: usize,
    }
    [
        LongSearch {
            q: "WhereareallthedentistsinAbercornIwouldlisomesomewhere",
            r: 0,
        },
        LongSearch {
            q: "Where are all the dentists in Abercorn I would like to find some somewhere",
            r: 1,
        },
        LongSearch {
            q: "Whereareallthedentists inAbercornIwouldliketofind some somewhere",
            r: 0,
        },
        LongSearch {
            q: "Whereareallthedentists in Bognor Regis Iwouldlike some somewhere",
            r: 1,
        },
        LongSearch {
            q: "Whereareallthedentists in Bognore Regis Iwouldlike some somewhere",
            r: 1,
        },
        LongSearch {
            q: "Whereareallthedentists in Bognoreregis Iwouldlike some somewhere",
            r: 1,
        },
        LongSearch {
            q: "Whereareallthedentists in BognoreRegistrar Iwouldlike some somewhere",
            r: 0,
        },
        LongSearch {
            q: "Whereareallthedentists some somewhere",
            r: 0,
        },
        LongSearch {
            q: "WhereareallthedentistsinAbercornIwouldlisomesomewhere",
            r: 0,
        },
    ]
    .iter()
    .for_each(|search| {
        let long_sentence = SearchTerm::from_raw_query(search.q.to_string(), None, 5, 3);
        let results = fake_data.search(&long_sentence);
        assert![
            results.len() == search.r,
            "Query: {}, results: {}, expected: {}",
            search.q,
            results.len(),
            search.r
        ];
    });
}

#[rstest]
fn should_search_punctuation(fake_data: &LocationsDb) {
    [
        "Armagh City",
        "Armagh City, Banbridge",
        "Armagh City, Banbridge and Craigavon",
    ]
    .iter()
    .for_each(|s| {
        let search_term = SearchTerm::from_raw_query(s.to_string(), None, 5, 3);
        let results = fake_data.search(&search_term);
        assert![results.len() == 1, "Found {}", results.len()];
        let armagh = results[0].0;
        assert![armagh == "ISO-3166-2-gb:abc"];

        let armagh_loc = &fake_data.all[&armagh];

        assert![armagh_loc.get_state() == "gb"];
        assert![armagh_loc.get_subdiv().unwrap() == "abc"];
    })
}

#[rstest]
fn should_search_generic(fake_data: &LocationsDb) {
    let search_term = SearchTerm::from_raw_query("One1".to_string(), None, 5, 3);
    let results = fake_data.search(&search_term);
    assert![results.len() == 1, "Found {}", results.len()];

    let my_one = results[0].0;
    assert![my_one == "MY-STANDARD-my:1"];

    let my_one_loc = &fake_data.all[&my_one];

    assert![my_one_loc.get_state() == "bg"];
    assert![my_one_loc.get_subdiv().unwrap() == "02"];
}
