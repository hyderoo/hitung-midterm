//! cargo run --example proj_local -- senate.wikitext house.wikitext
use hitung_midterm::{config::Reference, projection::*};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let r = Reference::load();
    let names = r.map.states.iter().map(|(c, s)| (s.name.clone(), c.clone())).collect();
    let s = std::fs::read_to_string(&a[1]).unwrap();
    let h = std::fs::read_to_string(&a[2]).unwrap();
    println!("senate {:?}", parse_senate(&s, &names));
    println!("house {:?}", parse_house(&h, &names));
    println!("totals {:?} {:?}", parse_totals(&s), parse_totals(&h));
}
