# Hitung Midterm

A live count page for the 3 November 2026 US midterm elections, written in Rust and deployed on Vercel. The UI is in Indonesian.

It shows **unofficial vote counts read directly from official state result feeds**. It does not call races, project winners or total up control of the Senate or the House.

## What it does

- Two views on one map: **Senat** (35 Senate contests) and **DPR** (435 House seats).
- Reads the "Media Export" JSON that the state election-night-reporting sites publish (Enhanced Voting format).
- Shows, per contest: candidates, vote counts, who is ahead on the votes counted so far ("unggul sementara"), margin, precincts reporting (when the feed provides it), and the source link.
- Shows the **source timestamp** (the `createdAt` the publisher stamped on the file) separately from the time the page was built, and flags a source that has not updated for more than 30 minutes.
- If a feed fails or cannot be parsed, that state shows "Gagal diperbarui" and no numbers. It never falls back to zero.
- States without a connected feed show "data belum tersedia". National totals and chamber control are always "Tidak tersedia" while coverage is partial.

## Coverage

Connected so far: Virginia and Georgia (Senate and House). Coverage is derived at runtime from `data/sources.json` and what each feed contains. Adding a state means adding one entry there if it publishes the same format.

| Reference data | Source |
| --- | --- |
| Which states hold a Senate contest on 3 Nov 2026 (33 regular + 2 special: FL, OH) | [Ballotpedia](https://ballotpedia.org/United_States_Senate_elections%2C_2026); regular seats cross-checked against Class II senators on [senate.gov](https://www.senate.gov/general/contact_information/senators_cfm.xml) |
| House seats per state (total 435) | [US Census Bureau, 2020 apportionment, Table 1](https://www2.census.gov/programs-surveys/decennial/2020/data/apportionment/apportionment-2020-table01.pdf) |
| Map geometry | [us-atlas](https://github.com/topojson/us-atlas) (Census cartographic boundaries) |
| Vote counts | Virginia Department of Elections ENR, Georgia Secretary of State ENR |

These files contain no election results. Results only ever come from the live feeds.

## How it works

`api/index.rs` is a Vercel Rust function. On each request it fetches the configured feeds concurrently, parses only the fields it needs with `serde`, and renders the page on the server (`src/render.rs`). `?format=json` returns the same data as JSON, `?view=dpr` selects the House view.

The response is cached at the Vercel edge (`s-maxage=120`, `stale-while-revalidate=300`), so the state servers are hit at most about once every two minutes however many people visit. Failures are cached for 20 seconds only.

## Develop

```sh
cargo test
# render the page from local copies of the feeds
cargo run --example render_local -- va.json ga.json senat > out.html
```

The tests use a small synthetic fixture (`tests/fixtures/test_only_export.json`, clearly labelled, not election data).

## Limits and caveats

- Counts are unofficial until each state certifies. They can change or be corrected.
- A vote lead is not a projection. There is no projection source connected, so "Proyeksi menang" is never shown.
- Precinct reporting comes from each feed's locality rows when present. Nothing is estimated.
- State sites update on their own schedule. Georgia's file, for example, can be hours old; the page shows that age.
- The Vercel Rust runtime is in beta.

## License

MIT for the code. Election data belongs to the respective state agencies. Public Sans (public/PublicSans.ttf) is under the SIL Open Font License 1.1.
