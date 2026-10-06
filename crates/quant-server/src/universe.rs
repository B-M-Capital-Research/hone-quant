//! The investment universe, derived from the honeclaw industry ontology.
//!
//! honeclaw's `skills/industry-map/references/industry-map.json` is the source of truth for
//! *which* companies and sectors exist (ten sectors under the "AI data center" root, each with
//! subtypes). Administrators can change membership live through an append-only edit log
//! (`data/industry_map/edits.json`); [`build_from_ontology`] replays the membership-relevant
//! operations of that log so hone-quant sees the same tree honeclaw serves.
//!
//! hone-quant never selects stocks: the strategy only weights the members of this universe.
//! The ontology's member `role` text is research context — explicitly *not* a rating — and is
//! stored only for display. A bundled snapshot (`config/universe.json`) ships with the binary;
//! `hone-quant universe sync` refreshes it, and every applied change is versioned in the database.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use deadpool_postgres::GenericClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const BUNDLED: &str = include_str!("../../../config/universe.json");
const OVERLAY: &str = include_str!("../../../config/universe-overlay.json");
/// Public location of the ontology in the honeclaw repository.
pub const ONTOLOGY_URL: &str = "https://raw.githubusercontent.com/B-M-Capital-Research/honeclaw/main/skills/industry-map/references/industry-map.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UniverseSource {
    pub kind: String,
    pub location: String,
    pub ontology_schema_version: Option<i64>,
    pub ontology_generated_at: Option<String>,
    pub edits_applied: usize,
    pub built_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorDef {
    pub id: String,
    pub name_zh: String,
    pub name_en: String,
    pub summary_zh: String,
    pub summary_en: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetDef {
    pub symbol: String,
    pub name_zh: String,
    pub name_en: String,
    pub sector: String,
    pub subtype_id: String,
    pub subtype_zh: String,
    pub subtype_en: String,
    pub also_in: Vec<String>,
    pub role_zh: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkDef {
    pub symbol: String,
    pub name_zh: String,
    pub name_en: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UniverseFile {
    pub schema_version: u32,
    pub source: UniverseSource,
    pub sectors: Vec<SectorDef>,
    pub assets: Vec<AssetDef>,
    pub benchmarks: Vec<BenchmarkDef>,
}

impl UniverseFile {
    /// Hash of the tradable content (sectors and members), used to detect changes.
    pub fn content_hash(&self) -> String {
        let canonical = serde_json::json!({ "sectors": self.sectors, "assets": self.assets });
        hex::encode(Sha256::digest(canonical.to_string().as_bytes()))
    }

    pub fn symbols(&self) -> Vec<String> {
        self.assets.iter().map(|a| a.symbol.clone()).collect()
    }

    pub fn validate(&self) -> Result<()> {
        if self.sectors.is_empty() || self.assets.is_empty() {
            bail!("universe has no sectors or no assets");
        }
        if self.sectors.len() > 10 {
            bail!(
                "universe has {} sectors; hone-quant is limited to ten",
                self.sectors.len()
            );
        }
        let sector_ids: BTreeSet<&str> = self.sectors.iter().map(|s| s.id.as_str()).collect();
        let mut seen = BTreeSet::new();
        for asset in &self.assets {
            if !seen.insert(asset.symbol.as_str()) {
                bail!("duplicate symbol {}", asset.symbol);
            }
            if !sector_ids.contains(asset.sector.as_str()) {
                bail!(
                    "{} references unknown sector {}",
                    asset.symbol,
                    asset.sector
                );
            }
            if asset.symbol.is_empty()
                || !asset
                    .symbol
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '.' || c == '-')
            {
                bail!("invalid symbol {:?}", asset.symbol);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OverlaySector {
    name_zh: Option<String>,
    name_en: Option<String>,
    summary_en: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OverlayAsset {
    name_zh: Option<String>,
    name_en: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Overlay {
    #[serde(default)]
    sectors: BTreeMap<String, OverlaySector>,
    #[serde(default)]
    subtypes: BTreeMap<String, String>,
    #[serde(default)]
    assets: BTreeMap<String, OverlayAsset>,
    #[serde(default)]
    primary_sector: BTreeMap<String, String>,
    #[serde(default)]
    benchmarks: Vec<BenchmarkDef>,
}

pub fn overlay() -> Overlay {
    serde_json::from_str(OVERLAY).expect("bundled overlay is valid JSON")
}

pub fn bundled() -> UniverseFile {
    serde_json::from_str(BUNDLED).expect("bundled universe is valid JSON")
}

/// The bundled universe, or a replacement file when configured.
pub fn load(path: Option<&Path>) -> Result<UniverseFile> {
    let universe = match path {
        None => bundled(),
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read universe file {}", path.display()))?;
            serde_json::from_str(&text)
                .with_context(|| format!("{} is not a universe file", path.display()))?
        }
    };
    universe.validate()?;
    Ok(universe)
}

// ---------------------------------------------------------------------------------------------
// Ontology → universe
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OntMember {
    symbol: String,
    name: String,
    role: String,
}

#[derive(Debug, Clone)]
struct OntSubtype {
    id: String,
    name: String,
    members: Vec<String>,
}

#[derive(Debug, Clone)]
struct OntIndustry {
    id: String,
    name: String,
    one_liner: String,
    members: Vec<OntMember>,
    subtypes: Vec<OntSubtype>,
}

fn str_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn parse_member(value: &Value) -> Option<OntMember> {
    let symbol = value.get("symbol")?.as_str()?.trim().to_ascii_uppercase();
    if symbol.is_empty() {
        return None;
    }
    Some(OntMember {
        symbol,
        name: str_of(value, "name"),
        role: str_of(value, "role"),
    })
}

fn parse_subtype(value: &Value) -> Option<OntSubtype> {
    Some(OntSubtype {
        id: value.get("id")?.as_str()?.to_string(),
        name: str_of(value, "name"),
        members: value
            .get("members")
            .and_then(Value::as_array)
            .map(|m| {
                m.iter()
                    .filter_map(|s| s.as_str().map(|s| s.to_ascii_uppercase()))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn parse_industries(ontology: &Value) -> Result<Vec<OntIndustry>> {
    let industries = ontology
        .get("industries")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("ontology has no `industries` array"))?;
    Ok(industries
        .iter()
        .filter_map(|ind| {
            Some(OntIndustry {
                id: ind.get("id")?.as_str()?.to_string(),
                name: str_of(ind, "name"),
                one_liner: str_of(ind, "one_liner"),
                members: ind
                    .get("members")
                    .and_then(Value::as_array)
                    .map(|m| m.iter().filter_map(parse_member).collect())
                    .unwrap_or_default(),
                subtypes: ind
                    .pointer("/valuation/subtypes")
                    .and_then(Value::as_array)
                    .map(|s| s.iter().filter_map(parse_subtype).collect())
                    .unwrap_or_default(),
            })
        })
        .collect())
}

/// Replays the membership-relevant operations of honeclaw's edit log. Operations that point at
/// missing industries or members are skipped, exactly as honeclaw does when it replays the log.
fn apply_edits(industries: &mut Vec<OntIndustry>, edits: &Value) -> usize {
    let Some(list) = edits.get("edits").and_then(Value::as_array) else {
        return 0;
    };
    let mut applied = 0;
    for edit in list {
        let Some(industry_id) = edit.get("industry").and_then(Value::as_str) else {
            continue;
        };
        let Some(op) = edit.get("op") else { continue };
        let kind = op.get("kind").and_then(Value::as_str).unwrap_or_default();
        if kind == "add_industry" {
            if let Some(new) = op.get("industry") {
                let id = str_of(new, "id");
                if !id.is_empty() && !industries.iter().any(|i| i.id == id) {
                    industries.push(OntIndustry {
                        id,
                        name: str_of(new, "name"),
                        one_liner: str_of(new, "one_liner"),
                        members: Vec::new(),
                        subtypes: Vec::new(),
                    });
                    applied += 1;
                }
            }
            continue;
        }
        if kind == "remove_industry" {
            let before = industries.len();
            industries.retain(|i| i.id != industry_id);
            applied += usize::from(industries.len() < before);
            continue;
        }
        let Some(industry) = industries.iter_mut().find(|i| i.id == industry_id) else {
            continue;
        };
        let ok = match kind {
            "add_member" => op
                .get("member")
                .and_then(parse_member)
                .is_some_and(|member| {
                    if industry.members.iter().any(|m| m.symbol == member.symbol) {
                        false
                    } else {
                        industry.members.push(member);
                        true
                    }
                }),
            "remove_member" => op
                .get("symbol")
                .and_then(Value::as_str)
                .is_some_and(|symbol| {
                    let symbol = symbol.to_ascii_uppercase();
                    let before = industry.members.len();
                    industry.members.retain(|m| m.symbol != symbol);
                    for subtype in industry.subtypes.iter_mut() {
                        subtype.members.retain(|m| *m != symbol);
                    }
                    industry.members.len() < before
                }),
            "set_member_role" => {
                let symbol = str_of(op, "symbol").to_ascii_uppercase();
                match industry.members.iter_mut().find(|m| m.symbol == symbol) {
                    Some(member) => {
                        member.role = str_of(op, "role");
                        true
                    }
                    None => false,
                }
            }
            "set_member_subtype" => {
                let symbol = str_of(op, "symbol").to_ascii_uppercase();
                let target = str_of(op, "subtype");
                if industry.subtypes.iter().any(|s| s.id == target) {
                    for subtype in industry.subtypes.iter_mut() {
                        subtype.members.retain(|m| *m != symbol);
                        if subtype.id == target {
                            subtype.members.push(symbol.clone());
                        }
                    }
                    true
                } else {
                    false
                }
            }
            "upsert_subtype" => op
                .get("subtype")
                .and_then(parse_subtype)
                .is_some_and(|new| {
                    match industry.subtypes.iter_mut().find(|s| s.id == new.id) {
                        Some(existing) => *existing = new,
                        None => industry.subtypes.push(new),
                    }
                    true
                }),
            "remove_subtype" => {
                let id = str_of(op, "id");
                let before = industry.subtypes.len();
                industry.subtypes.retain(|s| s.id != id);
                industry.subtypes.len() < before
            }
            "set_field" if str_of(op, "field") == "one_liner" => {
                industry.one_liner = str_of(op, "value");
                true
            }
            _ => false,
        };
        applied += usize::from(ok);
    }
    applied
}

/// "英伟达（NVIDIA）" → "英伟达"; "应用材料 Applied Materials" → "应用材料".
fn short_zh_name(name: &str) -> String {
    let cut = name.split(['（', '(']).next().unwrap_or(name).trim();
    let has_cjk = cut.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    if has_cjk {
        // Drop a trailing Latin alias after the Chinese name.
        let mut out = String::new();
        for word in cut.split_whitespace() {
            if word.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) || out.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(word);
            } else {
                break;
            }
        }
        out
    } else {
        cut.to_string()
    }
}

pub fn build_from_ontology(
    ontology: &Value,
    edits: Option<&Value>,
    overlay: &Overlay,
    location: &str,
) -> Result<UniverseFile> {
    let mut industries = parse_industries(ontology)?;
    let edits_applied = edits.map(|e| apply_edits(&mut industries, e)).unwrap_or(0);

    let sectors: Vec<SectorDef> = industries
        .iter()
        .enumerate()
        .map(|(i, ind)| {
            let o = overlay.sectors.get(&ind.id).cloned().unwrap_or_default();
            SectorDef {
                id: ind.id.clone(),
                name_zh: o.name_zh.unwrap_or_else(|| ind.name.clone()),
                name_en: o.name_en.unwrap_or_else(|| ind.id.clone()),
                summary_zh: ind.one_liner.clone(),
                summary_en: o.summary_en.unwrap_or_default(),
                sort_order: i as i32,
            }
        })
        .collect();

    // Every symbol, in ontology order, with all the industries it appears in.
    let mut order: Vec<String> = Vec::new();
    let mut appearances: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, ind) in industries.iter().enumerate() {
        for member in &ind.members {
            if !appearances.contains_key(&member.symbol) {
                order.push(member.symbol.clone());
            }
            appearances
                .entry(member.symbol.clone())
                .or_default()
                .push(i);
        }
    }

    let mut assets = Vec::new();
    for (sort, symbol) in order.iter().enumerate() {
        let found = &appearances[symbol];
        let primary_idx = overlay
            .primary_sector
            .get(symbol)
            .and_then(|id| found.iter().copied().find(|&i| industries[i].id == *id))
            .unwrap_or(found[0]);
        let industry = &industries[primary_idx];
        let member = industry
            .members
            .iter()
            .find(|m| &m.symbol == symbol)
            .expect("member present in its industry");
        let subtype = industry
            .subtypes
            .iter()
            .find(|s| s.members.contains(symbol));
        let o = overlay.assets.get(symbol).cloned().unwrap_or_default();
        let ascii_name = member.name.is_ascii().then(|| member.name.clone());
        assets.push(AssetDef {
            symbol: symbol.clone(),
            name_zh: o.name_zh.unwrap_or_else(|| short_zh_name(&member.name)),
            name_en: o.name_en.or(ascii_name).unwrap_or_else(|| symbol.clone()),
            sector: industry.id.clone(),
            subtype_id: subtype.map(|s| s.id.clone()).unwrap_or_default(),
            subtype_zh: subtype.map(|s| s.name.clone()).unwrap_or_default(),
            subtype_en: subtype
                .and_then(|s| overlay.subtypes.get(&s.id).cloned())
                .unwrap_or_default(),
            also_in: found
                .iter()
                .filter(|&&i| i != primary_idx)
                .map(|&i| industries[i].id.clone())
                .collect(),
            role_zh: member.role.clone(),
            sort_order: sort as i32,
        });
    }

    let universe = UniverseFile {
        schema_version: 1,
        source: UniverseSource {
            kind: "honeclaw-ontology".into(),
            location: location.to_string(),
            ontology_schema_version: ontology.get("schema_version").and_then(Value::as_i64),
            ontology_generated_at: ontology
                .get("generated_at")
                .and_then(Value::as_str)
                .map(str::to_string),
            edits_applied,
            built_at: Utc::now().format("%Y-%m-%d").to_string(),
        },
        sectors,
        assets,
        benchmarks: overlay.benchmarks.clone(),
    };
    universe.validate()?;
    Ok(universe)
}

/// Reads a JSON document from a local path or an http(s) URL.
pub async fn read_json(location: &str) -> Result<Value> {
    if location.starts_with("http://") || location.starts_with("https://") {
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?
            .get(location)
            .send()
            .await
            .with_context(|| format!("cannot fetch {location}"))?
            .error_for_status()
            .with_context(|| format!("cannot fetch {location}"))?;
        Ok(response
            .json()
            .await
            .with_context(|| format!("{location} is not JSON"))?)
    } else {
        let text = tokio::fs::read_to_string(location)
            .await
            .with_context(|| format!("cannot read {location}"))?;
        serde_json::from_str(&text).with_context(|| format!("{location} is not JSON"))
    }
}

// ---------------------------------------------------------------------------------------------
// Database sync
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct UniverseChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// (symbol, from sector, to sector)
    pub moved: Vec<(String, String, String)>,
    pub sectors_added: Vec<String>,
    pub sectors_removed: Vec<String>,
    pub first_load: bool,
}

impl UniverseChanges {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.moved.is_empty()
            && self.sectors_added.is_empty()
            && self.sectors_removed.is_empty()
    }
}

/// Compares a universe with what the database currently holds (active rows only).
pub async fn diff(client: &impl GenericClient, universe: &UniverseFile) -> Result<UniverseChanges> {
    let rows = client
        .query("SELECT symbol, sector_id FROM assets WHERE is_active", &[])
        .await?;
    let current: BTreeMap<String, String> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
    let sector_rows = client
        .query("SELECT id FROM sectors WHERE is_active", &[])
        .await?;
    let current_sectors: BTreeSet<String> = sector_rows.iter().map(|r| r.get(0)).collect();
    let mut changes = UniverseChanges {
        first_load: current.is_empty(),
        ..UniverseChanges::default()
    };
    for asset in &universe.assets {
        match current.get(&asset.symbol) {
            None => changes.added.push(asset.symbol.clone()),
            Some(sector) if *sector != asset.sector => {
                changes
                    .moved
                    .push((asset.symbol.clone(), sector.clone(), asset.sector.clone()))
            }
            Some(_) => {}
        }
    }
    let incoming: BTreeSet<&str> = universe.assets.iter().map(|a| a.symbol.as_str()).collect();
    changes.removed = current
        .keys()
        .filter(|s| !incoming.contains(s.as_str()))
        .cloned()
        .collect();
    let incoming_sectors: BTreeSet<&str> = universe.sectors.iter().map(|s| s.id.as_str()).collect();
    changes.sectors_added = universe
        .sectors
        .iter()
        .filter(|s| !current_sectors.contains(&s.id))
        .map(|s| s.id.clone())
        .collect();
    changes.sectors_removed = current_sectors
        .iter()
        .filter(|s| !incoming_sectors.contains(s.as_str()))
        .cloned()
        .collect();
    Ok(changes)
}

/// Upserts the universe; assets and sectors that disappeared are deactivated (never deleted, so
/// history stays readable). Records a version row whenever the content hash changes.
pub async fn sync_to_db(
    client: &mut deadpool_postgres::Client,
    universe: &UniverseFile,
    actor: &str,
) -> Result<UniverseChanges> {
    universe.validate()?;
    let tx = client.transaction().await?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
        &[&crate::db::lock_key("universe")],
    )
    .await?;
    let changes = diff(&tx, universe).await?;
    let hash = universe.content_hash();
    let last_hash: Option<String> = tx
        .query_opt(
            "SELECT content_hash FROM universe_versions ORDER BY id DESC LIMIT 1",
            &[],
        )
        .await?
        .map(|r| r.get(0));

    for sector in &universe.sectors {
        tx.execute(
            "INSERT INTO sectors (id, name_zh, name_en, summary_zh, summary_en, sort_order, is_active, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, true, app_now())
             ON CONFLICT (id) DO UPDATE SET name_zh = EXCLUDED.name_zh, name_en = EXCLUDED.name_en,
               summary_zh = EXCLUDED.summary_zh, summary_en = EXCLUDED.summary_en,
               sort_order = EXCLUDED.sort_order, is_active = true, updated_at = app_now()",
            &[&sector.id, &sector.name_zh, &sector.name_en, &sector.summary_zh, &sector.summary_en, &sector.sort_order],
        )
        .await?;
    }
    let sector_ids: Vec<String> = universe.sectors.iter().map(|s| s.id.clone()).collect();
    tx.execute(
        "UPDATE sectors SET is_active = false, updated_at = app_now() WHERE is_active AND NOT (id = ANY($1))",
        &[&sector_ids],
    )
    .await?;
    for asset in &universe.assets {
        tx.execute(
            "INSERT INTO assets (symbol, name_zh, name_en, sector_id, subtype_id, subtype_zh, subtype_en, also_in, role_zh, sort_order, is_active, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, true, app_now())
             ON CONFLICT (symbol) DO UPDATE SET name_zh = EXCLUDED.name_zh, name_en = EXCLUDED.name_en,
               sector_id = EXCLUDED.sector_id, subtype_id = EXCLUDED.subtype_id, subtype_zh = EXCLUDED.subtype_zh,
               subtype_en = EXCLUDED.subtype_en, also_in = EXCLUDED.also_in, role_zh = EXCLUDED.role_zh,
               sort_order = EXCLUDED.sort_order, is_active = true, updated_at = app_now()",
            &[
                &asset.symbol,
                &asset.name_zh,
                &asset.name_en,
                &asset.sector,
                &asset.subtype_id,
                &asset.subtype_zh,
                &asset.subtype_en,
                &asset.also_in,
                &asset.role_zh,
                &asset.sort_order,
            ],
        )
        .await?;
    }
    let symbols = universe.symbols();
    tx.execute(
        "UPDATE assets SET is_active = false, updated_at = app_now() WHERE is_active AND NOT (symbol = ANY($1))",
        &[&symbols],
    )
    .await?;
    if last_hash.as_deref() != Some(hash.as_str()) {
        tx.execute(
            "INSERT INTO universe_versions (source, ontology_schema_version, ontology_generated_at, content_hash, changes, applied_by)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &universe.source.location,
                &universe.source.ontology_schema_version.map(|v| v as i32),
                &universe.source.ontology_generated_at,
                &hash,
                &serde_json::to_value(&changes)?,
                &actor,
            ],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ontology() -> Value {
        json!({
            "schema_version": 4,
            "generated_at": "2026-09-06",
            "industries": [
                {
                    "id": "ai-chip", "name": "AI 芯片", "one_liner": "芯片一句话",
                    "members": [
                        {"symbol": "NVDA", "name": "英伟达（NVIDIA）", "role": "平台"},
                        {"symbol": "AVGO", "name": "博通（Broadcom）", "role": "定制"}
                    ],
                    "valuation": {"subtypes": [
                        {"id": "gpu-platform", "name": "GPU平台", "members": ["NVDA"]},
                        {"id": "custom-and-network-silicon", "name": "定制与网络芯片", "members": ["AVGO"]}
                    ]}
                },
                {
                    "id": "optical", "name": "光通信与 AI 互连", "one_liner": "光一句话",
                    "members": [
                        {"symbol": "COHR", "name": "Coherent Corp.", "role": "激光"},
                        {"symbol": "AVGO", "name": "Broadcom Inc.", "role": "交换"}
                    ],
                    "valuation": {"subtypes": [
                        {"id": "connectivity-silicon", "name": "连接芯片", "members": ["AVGO"]},
                        {"id": "laser-components-vertical-integration", "name": "光源器件与垂直整合", "members": ["COHR"]}
                    ]}
                }
            ]
        })
    }

    #[test]
    fn builds_bilingual_universe_with_primary_sectors() {
        let universe = build_from_ontology(&ontology(), None, &overlay(), "test").unwrap();
        assert_eq!(universe.sectors.len(), 2);
        assert_eq!(universe.sectors[0].name_en, "AI Chips");
        assert_eq!(universe.sectors[1].summary_zh, "光一句话");
        let symbols: Vec<&str> = universe.assets.iter().map(|a| a.symbol.as_str()).collect();
        assert_eq!(symbols, vec!["NVDA", "AVGO", "COHR"]);
        let avgo = &universe.assets[1];
        assert_eq!(avgo.sector, "ai-chip");
        assert_eq!(avgo.also_in, vec!["optical".to_string()]);
        assert_eq!(avgo.subtype_id, "custom-and-network-silicon");
        assert_eq!(avgo.subtype_en, "Custom & networking silicon");
        assert_eq!(avgo.name_zh, "博通");
        assert_eq!(universe.source.ontology_schema_version, Some(4));
        assert_eq!(universe.benchmarks.len(), 3);
    }

    #[test]
    fn replays_membership_edits() {
        let edits = json!({"schema_version": 1, "edits": [
            {"at": "2026-09-10T00:00:00Z", "by": "u1", "industry": "optical",
             "op": {"kind": "add_member", "member": {"symbol": "lite", "name": "Lumentum Holdings", "role": "光源"}}},
            {"at": "2026-09-11T00:00:00Z", "by": "u1", "industry": "ai-chip",
             "op": {"kind": "remove_member", "symbol": "NVDA"}},
            {"at": "2026-09-12T00:00:00Z", "by": "u1", "industry": "optical",
             "op": {"kind": "set_member_subtype", "symbol": "LITE", "subtype": "laser-components-vertical-integration"}},
            {"at": "2026-09-12T00:00:00Z", "by": "u1", "industry": "missing",
             "op": {"kind": "remove_member", "symbol": "COHR"}}
        ]});
        let universe = build_from_ontology(&ontology(), Some(&edits), &overlay(), "test").unwrap();
        assert_eq!(universe.source.edits_applied, 3);
        assert!(!universe.assets.iter().any(|a| a.symbol == "NVDA"));
        let lite = universe.assets.iter().find(|a| a.symbol == "LITE").unwrap();
        assert_eq!(lite.sector, "optical");
        assert_eq!(lite.subtype_id, "laser-components-vertical-integration");
        assert_eq!(lite.name_en, "Lumentum");
    }

    #[test]
    fn short_chinese_names() {
        assert_eq!(short_zh_name("英伟达（NVIDIA）"), "英伟达");
        assert_eq!(short_zh_name("应用材料 Applied Materials"), "应用材料");
        assert_eq!(
            short_zh_name("Camtek（以色列公司，纳斯达克上市）"),
            "Camtek"
        );
        assert_eq!(short_zh_name("Coherent Corp."), "Coherent Corp.");
    }

    #[test]
    fn bundled_universe_is_valid_and_matches_the_ontology_shape() {
        let universe = bundled();
        universe.validate().unwrap();
        assert!(universe.sectors.len() <= 10);
        assert!(universe.assets.len() >= universe.sectors.len());
        for asset in &universe.assets {
            assert!(
                !asset.name_en.is_empty() && !asset.name_zh.is_empty(),
                "{}",
                asset.symbol
            );
        }
    }

    #[test]
    fn validation_rejects_bad_universes() {
        let mut universe = build_from_ontology(&ontology(), None, &overlay(), "test").unwrap();
        universe.assets[0].sector = "nope".into();
        assert!(universe.validate().is_err());
        let mut universe = build_from_ontology(&ontology(), None, &overlay(), "test").unwrap();
        universe.assets[1].symbol = universe.assets[0].symbol.clone();
        assert!(universe.validate().is_err());
    }

    #[tokio::test]
    async fn sync_versions_changes_and_deactivates_removed_members() {
        let Some(db) = crate::db::testing::TestDb::new(crate::market::DataSource::Demo).await
        else {
            eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
            return;
        };
        let mut client = db.pool.get().await.unwrap();
        let first = build_from_ontology(&ontology(), None, &overlay(), "test").unwrap();
        let changes = sync_to_db(&mut client, &first, "test").await.unwrap();
        assert!(changes.first_load);
        assert_eq!(changes.added.len(), 3);
        // Same content again: no new version.
        let again = sync_to_db(&mut client, &first, "test").await.unwrap();
        assert!(again.is_empty());
        let versions: i64 = client
            .query_one("SELECT count(*) FROM universe_versions", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(versions, 1);
        // Remove NVDA.
        let edits = json!({"edits": [{"industry": "ai-chip", "op": {"kind": "remove_member", "symbol": "NVDA"}}]});
        let second = build_from_ontology(&ontology(), Some(&edits), &overlay(), "test").unwrap();
        let changes = sync_to_db(&mut client, &second, "test").await.unwrap();
        assert_eq!(changes.removed, vec!["NVDA".to_string()]);
        let active: bool = client
            .query_one("SELECT is_active FROM assets WHERE symbol = 'NVDA'", &[])
            .await
            .unwrap()
            .get(0);
        assert!(!active);
        drop(client);
        db.drop().await;
    }
}
