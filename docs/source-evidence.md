# Source qualification evidence

Per-source verification record. Update when adding or re-qualifying a source.
All timestamps UTC unless noted. "Linkage" = the official state site page that
links to the results host.

## Active sources

| State | Format | Feed URL | Linkage evidence | Terms check | Verified |
| --- | --- | --- | --- | --- | --- |
| AZ | Enhanced Voting export | app.enhancedvoting.com/cdn/results/Arizona/export-110326GeneralElection.json | state results page links export (verified earlier) | no challenge | 2026-11-04 |
| GA | Enhanced Voting export | results.sos.ga.gov/cdn/results/Georgia/export-11032026GeneralAndSpecialElection.json | sos.ga.gov linkage (verified earlier) | no challenge | 2026-11-04 |
| ID | Enhanced Voting export | results.voteidaho.gov/cdn/results/id/export-nov2026.json | results.voteidaho.gov state-host page carries its own Media Export href to the same URL; vendor-host copy byte-identical (md5 checked) | no challenge | 2026-11-04 |
| MD | mdsbe HTML | elections.maryland.gov/elections/2026/General_results/gen_results_2026_4.html | elections.maryland.gov is the state board site | no challenge | 2026-11-04 |
| NC | ncsbe results file | er.ncsbe.gov/enr/20261103/data/results_0.txt | er.ncsbe.gov is the state board ENR host | no challenge | 2026-11-04 |
| TX | txsos HTML | electionresults.sos.state.tx.us/results.html | sos.state.tx.us host | no challenge; feed lagging (05:42Z) | 2026-11-04 |
| OR | orsos HTML | results.oregonvotes.gov/resultsSW.aspx?type=FED&map=CTY | oregonvotes.gov is the state site | no challenge | 2026-11-04 |
| UT | Enhanced Voting export | electionresults.utah.gov/cdn/results/Utah/export-General11032026.json | electionresults.utah.gov Media Export linkage verified | no challenge | 2026-11-04 |
| WA | Enhanced Voting export | (see sources.json) | sos.wa.gov linkage verified | feed lagging (05:53Z) | 2026-11-04 |
| NM | orsos HTML | electionresults.sos.nm.gov/resultsSW.aspx?type=FED&map=CTY | www.sos.nm.gov links electionresults.sos.nm.gov | robots.txt 404; no terms page on sos.nm.gov | 2026-11-04 |
| SD | orsos HTML | electionresults.sd.gov/resultsSW.aspx?type=SWR&map=CTY | sdsos.gov ElectionNightResults.aspx links electionresults.sd.gov/Default.aspx | robots.txt 404 | 2026-11-04 |
| NE | orsos HTML (2 pages) | electionresults.nebraska.gov/resultsSW.aspx?type=SW&map=CTY + ?type=CG&map=DIST | sos.nebraska.gov links electionresults.nebraska.gov | robots.txt 404 | 2026-11-04 |
| AL | alsos HTML | www2.alabamavotes.gov/electionNight/statewideResultsByContest.aspx?ecode=1001305 | sos.alabama.gov /alabama-votes/voter/enr-current-list links www2.alabamavotes.gov electionNight pages (same URL shape) | robots.txt 404; page timestamp CT | 2026-11-04 |
| HI | hisummary TSV | elections.hawaii.gov/wp-content/results/2026 General/summary.txt | elections.hawaii.gov is the state Office of Elections; page links the files | Last-Modified used as source time | 2026-11-04 |
| VT | vtenr JSON (2-step) | static.electionresults.vermont.gov/elections/elections.json | sos.vermont.gov elections-results-data page links https://electionresults.vermont.gov/ | robots.txt 200, nothing barring this use; lastUpdatedDate is ET | 2026-11-04 |
| IN | inenr JSON (3-step) | enr.indianavoters.in.gov/site/data/settings.json | enr.indianavoters.in.gov is a subdomain of the official state voter portal indianavoters.in.gov | no challenge; WriteTime is ET | 2026-11-04 |

## Suspended

- VA: enr.elections.virginia.gov returned HTTP 403 to fetches from the hosting
  provider on 2026-11-04 09:09Z. Not retried; reopens only on a new external
  signal or a verified supported route.

## Party-label notes (official feeds' own labels)

- SD Senate: Brian L Bengs is labeled "Independent" by the SD feed.
- NE Senate: Dan Osborn is labeled "By Petition" by the NE feed.
- HI/IN single-letter parties (D/R) are mapped; anything else is Lainnya.
- TX feed's polling_pct reads 0 on districts with votes; meaning unknown, so
  it is shown only as a partial-reporting label, never a reporting fraction.

## Deferred leads

- WY: sos.wyo.gov/Elections/Docs/2026/2026GeneralResults.aspx posts PDF only;
  needs a PDF extraction step.
- SC: scvotes.gov/2026-general-election-results posts timestamped XLSX
  snapshots (latest seen 12:00 AM ET); no live 2026 general ENR page found on
  enr-scvotes.org.
- Not yet examined: NY, MA, NJ, IL, KY, LA, TN, AK, ND, MS, AR, KS, OK, DE.
- Stay-out: NV/PA/CA (Incapsula), MN (bot ban), VA (403), CO/FL/WI/IA/OH/MO/MI
  (no state linkage found), RI (stale + no linkage).

## Known data notes

- NE/SD Clarity pages carry federal contests on separate URLs (SW/SWR vs CG);
  the fetch concatenates pages before one parse.
- Clarity pages wrap "Results last updated" in span tags (NM/SD/NE) or render
  it plain (OR); the parser tolerates both.
