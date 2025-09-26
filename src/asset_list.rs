use anyhow::anyhow;
use derive_more::{Debug, Display};
use pest::{Parser, iterators::Pair};
use pest_derive::Parser;

#[derive(Parser)]
#[grammar_inline = r#"
Input = {
    SOI ~ List ~ EOI
}

List = {
    (Range | AssetId) ~ ( "," ~ (Range | AssetId) )*
}

Range = {
    AssetId ~ "--" ~ AssetId
}

AssetId = ${
    AssetIdComp ~ "-" ~ AssetIdComp
}

AssetIdComp = @{ ASCII_DIGIT{3} }

WHITESPACE = _{ " " }
"#]
struct AssetListParser;

#[derive(Copy, Clone, Display, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[display("{_0:03}-{_1:03}")]
#[debug("{_0:03}-{_1:03}")]
pub struct AssetId(u16, u16);

impl AssetId {
    pub fn increment(&mut self) {
        self.1 += 1;
        if self.1 > 999 {
            self.1 = 0;
            self.0 += 1;
        }
    }
}

#[derive(Debug)]
pub enum ListEntry {
    Range { from: AssetId, to: AssetId },
    Id(AssetId),
}

pub struct ListEntryIter {
    at: Option<AssetId>,
    entry: ListEntry,
}

impl IntoIterator for ListEntry {
    type Item = AssetId;
    type IntoIter = ListEntryIter;

    fn into_iter(self) -> Self::IntoIter {
        ListEntryIter {
            at: None,
            entry: self,
        }
    }
}

impl Iterator for ListEntryIter {
    type Item = AssetId;

    fn next(&mut self) -> Option<Self::Item> {
        match self.entry {
            ListEntry::Id(id) => {
                if self.at.is_none() {
                    self.at = Some(id);
                    Some(id)
                } else {
                    None
                }
            }
            ListEntry::Range { from, to } => {
                if let Some(at) = &mut self.at {
                    at.increment();
                    if *at > to { None } else { Some(*at) }
                } else {
                    self.at = Some(from);
                    Some(from)
                }
            }
        }
    }
}

pub trait Validate {
    fn validate(&self) -> Result<(), anyhow::Error>;
}

impl Validate for Vec<ListEntry> {
    fn validate(&self) -> Result<(), anyhow::Error> {
        for item in self {
            if let ListEntry::Range { from, to } = item
                && to < from
            {
                Err(anyhow!(
                    "The start of a range must be smaller than the end of a range!"
                ))?;
            }
        }

        Ok(())
    }
}

#[allow(
    clippy::result_large_err,
    reason = "error is from pest and contains useful info"
)]
pub fn parse<S: AsRef<str>>(input: S) -> Result<Vec<ListEntry>, pest::error::Error<Rule>> {
    let r = AssetListParser::parse(Rule::Input, input.as_ref())?
        .next()
        .unwrap();
    assert_eq!(r.as_rule(), Rule::Input);
    let r = r.into_inner().next().unwrap();
    assert_eq!(r.as_rule(), Rule::List);

    let mut list = vec![];
    for p in r.into_inner() {
        list.push(parse_range_or_id(p));
    }
    Ok(list)
}

fn parse_range_or_id(p: Pair<'_, Rule>) -> ListEntry {
    match p.as_rule() {
        Rule::AssetId => ListEntry::Id(parse_id(p)),
        Rule::Range => {
            let mut i = p.into_inner();
            let from = i.next().unwrap();
            let to = i.next().unwrap();
            assert_eq!(from.as_rule(), Rule::AssetId);
            assert_eq!(to.as_rule(), Rule::AssetId);

            ListEntry::Range {
                from: parse_id(from),
                to: parse_id(to),
            }
        }
        _ => panic!(
            "parse_range_or_id must be sent a pair that is not either a Range or an AssetId, was {:?}",
            p.as_rule()
        ),
    }
}

fn parse_id(p: Pair<'_, Rule>) -> AssetId {
    let mut i = p.into_inner();
    let comp_1 = i.next().unwrap();
    let comp_2 = i.next().unwrap();
    assert_eq!(comp_1.as_rule(), Rule::AssetIdComp);
    assert_eq!(comp_2.as_rule(), Rule::AssetIdComp);

    let comp_1: u16 = comp_1.as_str().parse().unwrap();
    let comp_2: u16 = comp_2.as_str().parse().unwrap();

    AssetId(comp_1, comp_2)
}
