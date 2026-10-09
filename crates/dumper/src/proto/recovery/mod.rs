use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::output::{ProtoItem, TypeToItemMap, short_name, snake_field};

pub(super) mod accessor;
pub(super) mod flow;
pub(super) mod response;
pub(super) mod response_names;
pub(super) mod validation;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Hint,
    Structural,
    Native,
    Metadata,
}

#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub symbol: String,
    pub name: String,
    pub original_name: Option<String>,
    pub source: String,
    pub method: Option<String>,
    pub rva: Option<usize>,
    pub evidence: Evidence,
}

// Legacy collectors retain all observations, including disagreements inside one pass.
#[derive(Clone, Debug, Default)]
pub struct Hints {
    pub entries: Vec<Candidate>,
    context: Option<(String, usize)>,
}

impl Hints {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn context(&mut self, method: impl Into<String>, rva: usize) {
        self.context = Some((method.into(), rva));
    }
    #[track_caller]
    pub fn insert(&mut self, symbol: String, name: String) {
        let caller = std::panic::Location::caller();
        self.entries.push(Candidate {
            symbol,
            name,
            original_name: None,
            source: format!("{}:{}", caller.file(), caller.line()),
            method: self.context.as_ref().map(|(m, _)| m.clone()),
            rva: self.context.as_ref().map(|(_, r)| *r).filter(|r| *r != 0),
            evidence: Evidence::Native,
        });
    }
    pub fn extend(&mut self, other: Self) {
        self.entries.extend(other.entries);
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.entries.iter().map(|c| (&c.symbol, &c.name))
    }
    pub fn values(&self) -> impl Iterator<Item = &String> {
        self.entries.iter().map(|c| &c.name)
    }
    pub fn unique(&self) -> HashMap<String, String> {
        let mut groups = BTreeMap::<&String, BTreeSet<&String>>::new();
        for c in self.entries.iter().filter(|c| c.evidence > Evidence::Hint) {
            groups.entry(&c.symbol).or_default().insert(&c.name);
        }
        groups
            .into_iter()
            .filter_map(|(key, names)| {
                (names.len() == 1).then(|| (key.clone(), (*names.first().unwrap()).clone()))
            })
            .collect()
    }

    pub fn advisory(&mut self, reason: &str) {
        for c in &mut self.entries {
            c.evidence = Evidence::Hint;
            c.source.push_str(&format!(" ({reason})"));
        }
    }
}

pub fn field_key(owner: &str, number: u32) -> String {
    format!("{owner}#{number}")
}

pub fn readable(name: &str) -> bool {
    let name = short_name(name);
    valid_name(name)
        && !super::util::is_obf(name)
        && !name.starts_with("unk_")
        && !name.starts_with("Unknown")
}

pub(super) fn readable_type(name: &str) -> bool {
    readable(name)
        && !matches!(
            short_name(name).to_ascii_lowercase().as_str(),
            "receive" | "send" | "handle" | "parse" | "process" | "invoke" | "request" | "response"
        )
}

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct ShapeField {
    pub name: String,
    pub kind: String,
    pub oneof: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub(super) struct Shape {
    pub fields: BTreeMap<u32, ShapeField>,
    pub variants: Vec<(String, i32)>,
    pub is_enum: bool,
}

pub(super) type Schema = BTreeMap<String, Shape>;

pub(super) fn snapshot(items: &TypeToItemMap, names: &HashMap<String, String>) -> Schema {
    let reverse: HashMap<_, _> = names
        .iter()
        .map(|(old, new)| (new.as_str(), old.as_str()))
        .collect();
    items
        .values()
        .map(|item| match &*item.borrow() {
            ProtoItem::Enum(en) => (
                en.name.clone(),
                Shape {
                    variants: en.variants.clone(),
                    is_enum: true,
                    ..Default::default()
                },
            ),
            ProtoItem::Message(m) => {
                let fields = m
                    .fields
                    .iter()
                    .map(|f| (f, None))
                    .chain(
                        m.oneofs
                            .iter()
                            .flat_map(|o| o.fields.iter().map(move |f| (f, Some(o.name.clone())))),
                    )
                    .map(|(f, oneof)| {
                        (
                            f.number,
                            ShapeField {
                                name: f.name.clone(),
                                oneof,
                                kind: super::output::rewrite_type_tokens(&f.kind, |s| {
                                    reverse.get(s).copied().unwrap_or(s).to_owned()
                                }),
                            },
                        )
                    })
                    .collect();
                (
                    m.name.clone(),
                    Shape {
                        fields,
                        ..Default::default()
                    },
                )
            }
        })
        .collect()
}

#[derive(Serialize)]
struct Decision {
    candidates: Vec<Candidate>,
    selected: Option<String>,
    reason: String,
}

#[derive(Deserialize, Serialize)]
struct AcceptedMapping {
    kind: String,
    original: String,
    name: String,
}

#[derive(Serialize)]
pub(super) struct Report {
    format_version: u32,
    status: String,
    pub inputs: BTreeMap<String, String>,
    pub notes: Vec<String>,
    decisions: BTreeMap<String, Decision>,
    accepted_mappings: BTreeMap<String, AcceptedMapping>,
    counts: BTreeMap<String, usize>,
    pub error: Option<String>,
}

pub(super) struct Recovery {
    pub schema: Schema,
    pub hints: Hints,
    pub report: Report,
    finished: bool,
}

impl Recovery {
    pub fn new(
        items: &TypeToItemMap,
        names: &HashMap<String, String>,
        core_proto: &[u8],
    ) -> anyhow::Result<Self> {
        let mut result = Self {
            schema: snapshot(items, names),
            hints: Hints::new(),
            finished: false,
            report: Report {
                format_version: 1,
                status: "running".into(),
                inputs: BTreeMap::new(),
                notes: Vec::new(),
                decisions: BTreeMap::new(),
                accepted_mappings: BTreeMap::new(),
                counts: BTreeMap::new(),
                error: None,
            },
        };
        result.report.inputs.insert(
            "core_proto_sha256".into(),
            format!("{:x}", Sha256::digest(core_proto)),
        );
        for (owner, shape) in &result.schema {
            if readable(short_name(owner)) {
                result.hints.entries.push(candidate(
                    owner,
                    short_name(owner),
                    "client metadata",
                    Evidence::Metadata,
                ));
            }
            for (number, f) in &shape.fields {
                if readable(&f.name) {
                    result.hints.entries.push(candidate(
                        &field_key(owner, *number),
                        &f.name,
                        "client field metadata",
                        Evidence::Metadata,
                    ));
                }
            }
        }
        result.report.inputs.insert(
            "wire_model_sha256".into(),
            format!("{:x}", Sha256::digest(serde_json::to_vec(&result.schema)?)),
        );
        result.save()?;
        Ok(result)
    }

    pub fn add(&mut self, hints: Hints) {
        for mut c in hints.entries {
            if let Some(raw) = c.symbol.strip_prefix("legacy-field:") {
                let mut scoped = Vec::new();
                for (owner, shape) in &self.schema {
                    for (number, field) in &shape.fields {
                        if field.name != raw {
                            continue;
                        }
                        let key = field_key(owner, *number);
                        let corroborated = self.hints.entries.iter().any(|other| {
                            other.symbol == key
                                && other.evidence >= Evidence::Native
                                && snake_field(&other.name) == snake_field(&c.name)
                        });
                        let mut local = c.clone();
                        local.symbol = key;
                        local.evidence = if corroborated {
                            Evidence::Structural
                        } else {
                            Evidence::Hint
                        };
                        local.source.push_str(if corroborated {
                            " (independently corroborated in this message)"
                        } else {
                            " (no independent evidence in this message; not applied)"
                        });
                        scoped.push(local);
                    }
                }
                self.hints.entries.extend(scoped);
                continue;
            }
            if let Some((owner, number)) = c.symbol.rsplit_once('#') {
                if !number.parse::<u32>().ok().is_some_and(|n| {
                    self.schema
                        .get(owner)
                        .is_some_and(|s| s.fields.contains_key(&n))
                }) {
                    c.evidence = Evidence::Hint;
                    c.source
                        .push_str(" (field identity is absent from current wire model)");
                }
                self.hints.entries.push(c);
                continue;
            }
            if self.schema.contains_key(&c.symbol) {
                self.hints.entries.push(c);
                continue;
            }
            // A short type identity is usable only if exactly one declaration owns it.
            let matches: Vec<_> = self
                .schema
                .keys()
                .filter(|s| short_name(s) == c.symbol)
                .cloned()
                .collect();
            if let [owner] = matches.as_slice() {
                c.symbol = owner.clone();
            } else {
                c.evidence = Evidence::Hint;
                c.source.push_str(" (unscoped legacy name; not applied)");
            }
            self.hints.entries.push(c);
        }
    }

    pub fn resolve(&mut self) -> (HashMap<String, String>, HashMap<String, String>) {
        let mut grouped = BTreeMap::<String, Vec<Candidate>>::new();
        for c in &self.hints.entries {
            grouped.entry(c.symbol.clone()).or_default().push(c.clone());
        }
        let mut selected = HashMap::new();
        self.report.decisions.clear();
        for (key, mut candidates) in grouped {
            candidates.sort_by(|a, b| {
                (
                    &a.name,
                    &a.source,
                    &a.method,
                    a.rva,
                    a.evidence,
                    &a.original_name,
                )
                    .cmp(&(
                        &b.name,
                        &b.source,
                        &b.method,
                        b.rva,
                        b.evidence,
                        &b.original_name,
                    ))
            });
            candidates.dedup_by(|a, b| {
                a.name == b.name
                    && a.source == b.source
                    && a.method == b.method
                    && a.rva == b.rva
                    && a.evidence == b.evidence
                    && a.original_name == b.original_name
            });
            let field = key.contains('#');
            let names: BTreeSet<_> = candidates
                .iter()
                .filter(|c| {
                    c.evidence > Evidence::Hint
                        && if field {
                            readable(&c.name)
                        } else {
                            readable_type(&c.name)
                        }
                })
                .map(|c| {
                    if field {
                        snake_field(&c.name)
                    } else {
                        short_name(&c.name).to_owned()
                    }
                })
                .collect();
            let (name, reason) = if names.len() == 1 {
                (names.first().cloned(), "unique corroborated identity")
            } else if names.is_empty() {
                (None, "no applicable scoped evidence")
            } else {
                (None, "conflicting reliable names")
            };
            if let Some(name) = &name {
                selected.insert(key.clone(), name.clone());
            }
            self.report.decisions.insert(
                key,
                Decision {
                    candidates,
                    selected: name,
                    reason: reason.into(),
                },
            );
        }
        // Recompute until rollback no longer introduces a collision with an original name.
        loop {
            let mut namespaces = BTreeMap::<String, Vec<String>>::new();
            for (owner, shape) in &self.schema {
                let scope = owner.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
                let name = selected
                    .get(owner)
                    .map(String::as_str)
                    .unwrap_or(short_name(owner));
                namespaces
                    .entry(format!("scope:{scope}:{name}"))
                    .or_default()
                    .push(owner.clone());
                if shape.is_enum {
                    for (variant, _) in &shape.variants {
                        namespaces
                            .entry(format!("scope:{scope}:{variant}"))
                            .or_default()
                            .push(format!("reserved enum value {owner}.{variant}"));
                    }
                }
                let oneofs: BTreeSet<_> = shape
                    .fields
                    .values()
                    .filter_map(|f| f.oneof.as_ref())
                    .collect();
                for group in oneofs {
                    let group = snake_field(group.strip_suffix("Case").unwrap_or(group));
                    namespaces
                        .entry(format!("scope:{owner}:{group}"))
                        .or_default()
                        .push(format!("reserved oneof {owner}.{group}"));
                }
                for (number, field) in &shape.fields {
                    let key = field_key(owner, *number);
                    let name = selected
                        .get(&key)
                        .map(String::as_str)
                        .unwrap_or(&field.name);
                    namespaces
                        .entry(format!("scope:{owner}:{}", snake_field(name)))
                        .or_default()
                        .push(key);
                }
            }
            let mut changed = false;
            for keys in namespaces.values().filter(|v| v.len() > 1) {
                for key in keys {
                    if selected.remove(key).is_some() {
                        if let Some(d) = self.report.decisions.get_mut(key) {
                            d.selected = None;
                            d.reason = format!("name collision in scope: {}", keys.join(", "));
                        }
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut types = HashMap::new();
        let mut fields = HashMap::new();
        for (key, value) in selected {
            if key.contains('#') {
                fields.insert(key, value);
            } else if self.schema.contains_key(&key) {
                types.insert(key, value);
            }
        }
        (types, fields)
    }

    pub fn finish(
        &mut self,
        items: &TypeToItemMap,
        final_names: &HashMap<String, String>,
    ) -> anyhow::Result<()> {
        let after = snapshot(items, final_names);
        anyhow::ensure!(
            after.len() == self.schema.len(),
            "NT changed definition count"
        );
        let (types, fields) = self.resolve();
        self.report.accepted_mappings.clear();
        let mut scoped = BTreeMap::new();
        let mut global = BTreeMap::<String, BTreeSet<String>>::new();
        for (owner, before) in &self.schema {
            let actual = &after[owner];
            anyhow::ensure!(
                before.is_enum == actual.is_enum && before.variants == actual.variants,
                "NT changed enum {owner}"
            );
            anyhow::ensure!(
                before.fields.len() == actual.fields.len(),
                "NT changed fields of {owner}"
            );
            let expected = rename_path(owner, &types);
            anyhow::ensure!(
                final_names.get(owner).unwrap_or(owner) == &expected,
                "NT type decision differs from output: {owner}"
            );
            if expected != *owner {
                scoped.insert(owner.clone(), short_name(&expected).to_owned());
                self.report.accepted_mappings.insert(
                    owner.clone(),
                    AcceptedMapping {
                        kind: if before.is_enum { "enum" } else { "message" }.into(),
                        original: owner.clone(),
                        name: expected,
                    },
                );
            }
            for (number, f) in &before.fields {
                let a = actual
                    .fields
                    .get(number)
                    .context("NT changed a field number")?;
                anyhow::ensure!(
                    f.kind == a.kind && f.oneof == a.oneof,
                    "NT changed wire field {owner}#{number}"
                );
                let key = field_key(owner, *number);
                let expected = fields.get(&key).map(String::as_str).unwrap_or(&f.name);
                anyhow::ensure!(
                    snake_field(expected) == snake_field(&a.name),
                    "NT field decision differs from output: {key}"
                );
                global
                    .entry(f.name.clone())
                    .or_default()
                    .insert(snake_field(&a.name));
                if snake_field(&f.name) != snake_field(&a.name) {
                    self.report.accepted_mappings.insert(
                        key.clone(),
                        AcceptedMapping {
                            kind: "field".into(),
                            original: f.name.clone(),
                            name: snake_field(&a.name),
                        },
                    );
                    scoped.insert(key, snake_field(&a.name));
                }
            }
        }
        let mut legacy = BTreeMap::new();
        let mut method_names = BTreeMap::new();
        for (source, targets) in &global {
            if targets.len() == 1 {
                let name = targets.iter().next().unwrap();
                if *name != snake_field(source) {
                    legacy.insert(source.clone(), name.clone());
                }
            }
        }
        for (owner, name) in types {
            if name != short_name(&owner) {
                if self
                    .report
                    .decisions
                    .get(&owner)
                    .is_some_and(|d| d.candidates.iter().any(|c| c.source.contains("method_nt")))
                {
                    method_names.insert(owner.clone(), name.clone());
                }
                // Never let a field entry impersonate a type entry in the legacy format.
                if global
                    .get(&owner)
                    .is_some_and(|targets| targets.iter().any(|n| *n != name))
                {
                    legacy.remove(&owner);
                } else {
                    legacy.insert(owner, name);
                }
            }
        }
        for (label, is_enum) in [("messages", false), ("enums", true)] {
            self.report.counts.insert(
                format!("{label}_total"),
                self.schema
                    .values()
                    .filter(|s| s.is_enum == is_enum)
                    .count(),
            );
            self.report.counts.insert(
                format!("{label}_confirmed"),
                self.schema
                    .iter()
                    .filter(|(name, s)| {
                        s.is_enum == is_enum
                            && readable(short_name(final_names.get(*name).unwrap_or(*name)))
                    })
                    .count(),
            );
        }
        self.report.counts.insert(
            "fields_total".into(),
            self.schema.values().map(|s| s.fields.len()).sum(),
        );
        self.report
            .counts
            .insert("fields_confirmed".into(), fields.len());
        for kind in ["messages", "enums", "fields"] {
            self.report.counts.insert(
                format!("{kind}_unresolved"),
                self.report.counts[&format!("{kind}_total")]
                    - self.report.counts[&format!("{kind}_confirmed")],
            );
        }
        self.report.counts.insert(
            "unresolved_candidate_groups".into(),
            self.report
                .decisions
                .values()
                .filter(|d| d.selected.is_none())
                .count(),
        );
        self.report.notes.push("Wire invariants verified against the immutable client model; rendered proto validated by protox. Unresolved candidates are not applied.".into());
        self.report.status = "validated_pending_publication".into();
        self.save()?;
        write_lines("DUMP/nt.txt.partial", &legacy)?;
        write_lines("DUMP/nt.scoped.txt.partial", &scoped)?;
        write_lines("DUMP/method_nt.txt.partial", &method_names)?;
        self.finished = true;
        Ok(())
    }

    fn save(&self) -> anyhow::Result<()> {
        save_report(&self.report)
    }
}

impl Drop for Recovery {
    fn drop(&mut self) {
        if !self.finished {
            self.report.status = "failed".into();
            if self.report.error.is_none() {
                self.report.error =
                    Some("Name recovery interrupted; see dump.log for the stage error".into());
            }
            let _ = self.save();
        }
    }
}

pub(super) fn candidate(symbol: &str, name: &str, source: &str, evidence: Evidence) -> Candidate {
    Candidate {
        symbol: symbol.into(),
        name: name.into(),
        original_name: None,
        source: source.into(),
        method: None,
        rva: None,
        evidence,
    }
}

pub(super) fn rename_path(path: &str, names: &HashMap<String, String>) -> String {
    let mut prefix = String::new();
    path.split('.')
        .map(|part| {
            if !prefix.is_empty() {
                prefix.push('.');
            }
            prefix.push_str(part);
            names
                .get(&prefix)
                .map(|s| short_name(s))
                .unwrap_or(part)
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn save_report(report: &Report) -> anyhow::Result<()> {
    use std::io::Write;
    let mut file = fs::File::create("DUMP/nt-report.json.partial")?;
    serde_json::to_writer_pretty(&mut file, report)?;
    writeln!(file)?;
    file.sync_all()?;
    fs::rename("DUMP/nt-report.json.partial", "DUMP/nt-report.json")?;
    Ok(())
}

fn write_lines(path: &str, entries: &BTreeMap<String, String>) -> anyhow::Result<()> {
    use std::io::Write;
    let mut file = fs::File::create(path)?;
    for (key, name) in entries {
        writeln!(file, "{key} {name}")?;
    }
    file.sync_all()?;
    Ok(())
}

pub(crate) fn publish() -> anyhow::Result<()> {
    let path = "DUMP/nt-report.json";
    let mut report: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    anyhow::ensure!(
        report["status"] == "validated_pending_publication",
        "NT report is not ready for publication"
    );
    let mappings: BTreeMap<String, AcceptedMapping> =
        serde_json::from_value(report["accepted_mappings"].clone())?;
    for name in ["nt.txt", "nt.scoped.txt", "method_nt.txt"] {
        fs::rename(format!("DUMP/{name}.partial"), format!("DUMP/{name}"))?;
    }
    report["status"] = "completed".into();
    let unresolved = report["counts"]["unresolved_candidate_groups"]
        .as_u64()
        .unwrap_or(0);
    fs::write(
        "DUMP/nt-report.json.partial",
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::rename("DUMP/nt-report.json.partial", path)?;
    // Publication happens once, after proto and NT validation. Intermediate
    // resolve passes must not announce candidates that may later be rolled back.
    for (symbol, mapping) in mappings {
        if mapping.kind == "field" {
            log::info!(
                "[NT] [field] {} -> {} [{symbol}]",
                mapping.original,
                mapping.name
            );
        } else {
            log::info!(
                "[NT] [{}] {} -> {}",
                mapping.kind,
                mapping.original,
                mapping.name
            );
        }
    }
    if let Some(decisions) = report["decisions"].as_object() {
        for (symbol, decision) in decisions {
            let reason = decision["reason"].as_str().unwrap_or("");
            if decision["selected"].is_null()
                && (reason == "conflicting reliable names"
                    || reason.starts_with("name collision in scope:"))
            {
                let names: BTreeSet<_> = decision["candidates"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["evidence"] != "hint")
                    .filter_map(|c| c["name"].as_str())
                    .collect();
                log::warn!("[NT] [unresolved] {symbol}: {names:?}; {reason}");
            }
        }
    }
    if unresolved != 0 {
        log::warn!("NT: {unresolved} unresolved candidate groups; details: {path}");
    }
    Ok(())
}

pub(crate) fn begin() -> anyhow::Result<()> {
    save_report(&Report {
        format_version: 1,
        status: "running".into(),
        inputs: BTreeMap::new(),
        notes: Vec::new(),
        decisions: BTreeMap::new(),
        accepted_mappings: BTreeMap::new(),
        counts: BTreeMap::new(),
        error: None,
    })
}

pub(crate) fn record_failure(error: &anyhow::Error) -> anyhow::Result<()> {
    let mut report: serde_json::Value = serde_json::from_slice(&fs::read("DUMP/nt-report.json")?)?;
    report["status"] = "failed".into();
    report["error"] = format!("{error:#}").into();
    fs::write(
        "DUMP/nt-report.json.partial",
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::rename("DUMP/nt-report.json.partial", "DUMP/nt-report.json")?;
    Ok(())
}
