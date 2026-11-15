pub struct SeasonRow {
    pub season: &'static str,
    pub operation: &'static str,
    pub events: &'static [&'static str],
    pub build: &'static str,
}

const fn row(
    season: &'static str,
    operation: &'static str,
    events: &'static [&'static str],
    build: &'static str,
) -> SeasonRow {
    SeasonRow {
        season,
        operation,
        events,
        build,
    }
}

pub static SUPPORT: &[SeasonRow] = &[
    row("Y1S0", "Vanilla", &[], "8194013"),
    row("Y1S1", "Black Ice", &[], "8519860"),
    row("Y1S2", "Dust Line", &[], "9132097"),
    row("Y1S3", "Skull Rain", &[], "9860556"),
    row("Y1S4", "Red Crow", &[], "10211195"),
    row("Y2S1", "Velvet Shell", &[], "10751226"),
    row("Y2S2", "Health", &[], "11216230"),
    row("Y2S3", "Blood Orchid", &[], "11432634"),
    row("Y2S4", "White Noise", &[], "11580709"),
    row("Y3S1", "Chimera", &["Outbreak"], "11726982"),
    row("Y3S2", "Para Bellum", &[], "11965022"),
    row("Y3S3", "Grim Sky", &["Mad House"], "12362767"),
    row("Y3S4", "Wind Bastion", &[], "12512571"),
    row("Y4S1", "Burnt Horizon", &["Rainbow is Magic"], "12863847"),
    row("Y4S2", "Phantom Sight", &["Showdown"], "13147883"),
    row(
        "Y4S3",
        "Ember Rise",
        &["Doktor's Curse", "Money Heist"],
        "13632147",
    ),
    row("Y4S4", "Shifting Tides", &["Road To S.I. 2020"], "13777760"),
    row("Y5S1", "Void Edge", &["Grand Larceny"], "14303219"),
    row("Y5S2", "Steel Wave", &["M.U.T.E. Protocol"], "14664131"),
    row("Y5S3", "Shadow Legacy", &["Sugar Fright"], "15018155"),
    row("Y5S4", "Neon Dawn", &["Road To S.I. 2021"], "15241382"),
    row(
        "Y6S1",
        "Crimson Heist",
        &["Rainbow is Magic", "Apocalypse"],
        "15680437",
    ),
    row("Y6S2", "North Star", &["Containment"], "15772114"),
    row(
        "Y6S3",
        "Crystal Guard",
        &["Showdown", "Doktor's Curse"],
        "35130040",
    ),
    row(
        "Y6S4",
        "High Calibre",
        &["Snow Brawl", "Road To S.I. 2022"],
        "37046065",
    ),
    row("Y7S1", "Demon Veil", &["Rengoku"], "39785137"),
    row("Y7S2", "Vector Glare", &["M.U.T.E. Protocol"], "43489433"),
    row("Y7S3", "Brutal Swarm", &["Doktor's Curse"], "47165842"),
    row("Y7S4", "Solar Raid", &["Snow Brawl"], "50497889"),
    row(
        "Y8S1",
        "Commanding Force",
        &["Rainbow is Magic"],
        "55217154",
    ),
    row(
        "Y8S2",
        "Dread Factor",
        &["Rainbow is Magic", "Rengoku"],
        "58222837",
    ),
    row("Y8S3", "Heavy Mettle", &["Doktor's Curse"], "62486471"),
    row("Y8S4", "Deep Freeze", &["Freeze For All"], "67466841"),
    row("Y9S1", "Deadly Omen", &["Containment"], "69866602"),
    row("Y9S2", "New Blood", &["M.U.T.E. Protocol"], "72730050"),
    row("Y9S3", "Twin Shells", &["Doktor's Curse"], "79429095"),
    row("Y9S4", "Collision Point", &["Freeze For All"], "82873685"),
    row("Y10S1", "Prep Phase", &[], "87835624"),
    row("Y10S2", "Daybreak", &[], "91107972"),
    row("Y10S3", "High Stakes", &[], "97612398"),
    row("Y10S4", "Tenfold Pursuit", &[], "101173381"),
    row("Y11S1", "Silent Hunt", &[], "108519331"),
    row("Y11S2", "System Override", &[], "114706078"),
];

fn year(row: &SeasonRow) -> &'static str {
    row.season.split(['Y', 'S']).nth(1).unwrap_or_default()
}

pub fn years() -> impl Iterator<Item = (&'static str, &'static [SeasonRow])> {
    SUPPORT
        .chunk_by(|earlier, later| year(earlier) == year(later))
        .map(|rows| (year(&rows[0]), rows))
}

pub fn events(build: &str) -> &'static [&'static str] {
    let row = SUPPORT.iter().find(|row| row.build == build);
    row.map(|row| row.events).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_group_into_the_years_one_to_eleven() {
        let numbers: Vec<u32> = years().map(|(year, _)| year.parse().unwrap()).collect();
        assert_eq!(numbers, Vec::from_iter(1..=11));
        assert_eq!(SUPPORT.len(), 43);
    }

    #[test]
    fn every_build_names_one_row() {
        for row in SUPPORT {
            let named = SUPPORT.iter().filter(|other| other.build == row.build);
            assert_eq!(named.count(), 1, "{}", row.build);
        }
    }
}
