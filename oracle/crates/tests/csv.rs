use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct City {
    name: String,
    country: String,
    population: u64,
}

#[test]
fn rfc4180_quoting_and_embedded_newlines() {
    let data = "id,note\n1,\"hello, world\"\n2,\"she said \"\"hi\"\"\"\n3,\"line1\nline2\"\n";
    let mut rdr = csv::Reader::from_reader(data.as_bytes());
    let headers: Vec<String> = rdr.headers().unwrap().iter().map(str::to_owned).collect();
    assert_eq!(headers, vec!["id", "note"]);

    let rows: Vec<Vec<String>> = rdr
        .records()
        .map(|r| r.unwrap().iter().map(str::to_owned).collect())
        .collect();
    assert_eq!(
        rows,
        vec![
            vec!["1", "hello, world"],
            vec!["2", "she said \"hi\""],
            vec!["3", "line1\nline2"],
        ]
    );
}

#[test]
fn serde_csv_serialize_and_deserialize_roundtrip() {
    let cities = vec![
        City {
            name: "Zurich".into(),
            country: "CH".into(),
            population: 434_008,
        },
        City {
            name: "San Francisco, CA".into(),
            country: "US".into(),
            population: 808_437,
        },
        City {
            name: "Tokyo".into(),
            country: "JP".into(),
            population: 13_960_000,
        },
    ];

    let mut wtr = csv::Writer::from_writer(Vec::new());
    for city in &cities {
        wtr.serialize(city).unwrap();
    }
    let bytes = wtr.into_inner().unwrap();

    let mut rdr = csv::Reader::from_reader(bytes.as_slice());
    let parsed: Vec<City> = rdr.deserialize().map(|r| r.unwrap()).collect();
    assert_eq!(parsed, cities);
}

#[test]
fn custom_delimiter_and_column_mismatch_error() {
    let tsv = "a\tb\tc\n10\t20\t30\n";
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .from_reader(tsv.as_bytes());
    let rec = rdr.records().next().unwrap().unwrap();
    assert_eq!((&rec[0], &rec[1], &rec[2]), ("10", "20", "30"));

    let bad = "a,b\n1,2,3\n";
    let mut bad_rdr = csv::Reader::from_reader(bad.as_bytes());
    assert!(bad_rdr.records().next().unwrap().is_err());
}
