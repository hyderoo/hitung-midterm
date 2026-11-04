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
| SC | scxlsx snapshot | scvotes.gov/2026-general-election-results/ (latest timestamped Results.xlsx) | scvotes.gov is the state election commission site; page posts the xlsx snapshots itself | robots.txt allows /wp-content/ (disallows only calendar paths); xlsx Last-Modified is source time | 2026-11-04 |
| DE | desocrata JSON (Socrata open data) | data.delaware.gov/resource/yg7x-kgjc.json | elections.delaware.gov/results/enr/GE2026.html links this exact dataset/query | robots.txt allows /resource/ (disallows only browse/catalog paths); crawl-delay 1, our refresh is >=90s; no terms page | 2026-11-04 |
| ND | totalresults JSON API | api.resultsnd.sos.nd.gov (Election/Contest endpoints, client north-dakota) | sos.nd.gov/elections/election-results links resultsnd.sos.nd.gov as official results | robots.txt 404 (no robots file) on web and API hosts; no terms page found; open unauthenticated JSON | 2026-11-04 |
| AR | totalresults JSON API | enr-results-api.totalresults.com (client arkansas) | sos.arkansas.gov/elections/research/election-results links arkansas.tally-enr.com as official results | robots.txt 404 on API host; tally-enr.com/robots.txt WAF 403 (not a fetched disallow list - see reasoning below); no terms page found; open unauthenticated JSON | 2026-11-04 |
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
- LA: voterportal.sos.la.gov robots.txt explicitly disallows
  /ElectionResults/ElectionResults/Data* (the live results API). Not fetched
  further; the alternate S3 xlsx host has no fetchable robots and needs an
  xlsx parser, deferred.
- NY: flateau.elections.ny.gov and nyenr.elections.ny.gov both 403 (WAF).
- NJ: official night page links county pages on 4+ different systems (13 on
  Clarity + Passaic; Bergen/Camden/Sussex on county clerk sites), no statewide
  page. Aggregation would need per-county parsers; deferred.
- IL: elections.il.gov ElectionVoteTotals.aspx still lists only the 2026
  primary; no general files posted yet.
- KY: elect.ky.gov 2026 page has no general results link yet;
  vrsws.sos.ky.gov/liveresults is 403 (WAF).
- MS/AK/TN: no state ENR found (TN elections.tn.gov 403 to curl; Clarity
  roots 404 for MS/AK/TN).
- MA: no live ENR found (state posts certified results only).
- KS/OK Clarity roots are empty shells; no state pages.
- Stay-out: NV/PA/CA (Incapsula), MN (bot ban), VA (403), CO/FL/WI/IA/OH/MO/MI
  (no state linkage found), RI (stale + no linkage).

## Qualification reasoning: WAF 403 vs terms

- A 403 from a host's Web Application Firewall on one probed path (e.g.
  arkansas.tally-enr.com/robots.txt) is not a fetched robots.txt disallow and
  not terms text; it says nothing about the clean JSON endpoints the official
  site itself loads in every visitor's browser. Qualification for ND/AR rests
  on: official linkage from the SoS site, robots.txt absent (404) on the API
  origins actually fetched, no terms page found on the results sites, and
  unauthenticated public JSON. Blocked paths are not retried to evade the WAF.

## Known data notes

- NE/SD Clarity pages carry federal contests on separate URLs (SW/SWR vs CG);
  the fetch concatenates pages before one parse.
- Clarity pages wrap "Results last updated" in span tags (NM/SD/NE) or render
  it plain (OR); the parser tolerates both.
