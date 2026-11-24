//! LoRA Studio: building a training set from a catalog. `plan_build` is the single place that
//! turns a selection (tracks, hold-out, config) into the plan the website previews and that
//! creating a LoRA re-checks, so the two can never disagree.

use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    AppState,
    auth::Trainer,
    error::{AppError, AppResult},
    routes::{
        catalogs::{CatalogTrack, Scope, load_tracks},
        training::validate_config,
    },
};

/// Songs longer than this are cut to this many seconds when training.
const MAX_TRAIN_SECONDS: f64 = 240.0;

#[derive(Deserialize, Serialize, Clone)]
pub struct BuildRequest {
    pub name: String,
    #[serde(default)]
    pub keyword: String,
    pub test_prompt: String,
    pub track_ids: Vec<String>,
    pub holdout_count: i64,
    pub holdout_seed: i64,
    #[serde(default)]
    pub voice_confirmed: Vec<String>,
    pub base_model_id: String,
    pub rank: i64,
    pub alpha: i64,
    pub learning_rate: f64,
    pub epochs: i64,
    pub gradient_accumulation: i64,
    pub save_every: i64,
    pub warmup_steps: i64,
    #[serde(default)]
    pub seed: Option<i64>,
    pub max_minutes: i64,
}

#[derive(Serialize)]
pub struct BuildTrack {
    pub id: String,
    pub filename: String,
    pub duration_s: Option<f64>,
    pub model_key: Option<String>,
    pub vocals: String,
    pub caption: String,
}

#[derive(Serialize)]
pub struct ModelCount {
    pub model_key: String,
    pub name: String,
    pub count: i64,
}

#[derive(Serialize)]
pub struct BuildPlan {
    pub ready: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub train: Vec<BuildTrack>,
    pub held_out: Vec<BuildTrack>,
    pub missing_caption: Vec<BuildTrack>,
    pub voice_unconfirmed: Vec<BuildTrack>,
    pub no_tempo_key: Vec<BuildTrack>,
    /// Sum of `min(duration, 240)` over train songs that have a duration.
    pub train_seconds: f64,
    pub unknown_duration: i64,
    pub models: Vec<ModelCount>,
    pub steps_per_epoch: i64,
    pub total_steps: i64,
    pub checkpoint_epochs: Vec<i64>,
    pub eval_renders: i64,
    pub name_taken: bool,
}

fn build_track(t: &CatalogTrack) -> BuildTrack {
    BuildTrack {
        id: t.id.clone(),
        filename: t.filename.clone(),
        duration_s: t.duration_s,
        model_key: t.caption.as_ref().map(|c| c.model_key.clone()),
        vocals: t
            .caption
            .as_ref()
            .map(|c| c.vocals.clone())
            .unwrap_or_default(),
        caption: t
            .caption
            .as_ref()
            .map(|c| c.caption.clone())
            .unwrap_or_default(),
    }
}

/// Deterministic shuffle source, so the same seed always holds out the same songs.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Ids to hold out: `count` of `eligible` (sorted by id first), picked by a seeded Fisher-Yates.
fn pick_holdout(mut eligible: Vec<String>, count: usize, seed: i64) -> HashSet<String> {
    eligible.sort();
    let mut rng = SplitMix64(seed as u64);
    for i in (1..eligible.len()).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        eligible.swap(i, j);
    }
    eligible.into_iter().take(count).collect()
}

fn keyword_ok(keyword: &str) -> bool {
    keyword.chars().count() <= 40
        && keyword
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub(crate) async fn plan_build(
    db: &SqlitePool,
    catalog_id: &str,
    req: &BuildRequest,
) -> AppResult<BuildPlan> {
    let found: Option<(String,)> =
        sqlx::query_as("SELECT id FROM catalogs WHERE id = ? AND deleted_at IS NULL")
            .bind(catalog_id)
            .fetch_optional(db)
            .await?;
    if found.is_none() {
        return Err(AppError::not_found("catalog"));
    }
    validate_config(
        req.rank,
        req.alpha,
        req.learning_rate,
        req.epochs,
        req.gradient_accumulation,
        req.save_every,
        req.warmup_steps,
        req.max_minutes,
    )?;

    let selected: HashSet<&str> = req.track_ids.iter().map(String::as_str).collect();
    if req.track_ids.is_empty() || selected.len() != req.track_ids.len() {
        return Err(AppError::validation("Chọn ít nhất 1 bài, không trùng nhau"));
    }
    let all = load_tracks(db, Scope::Catalog(catalog_id)).await?;
    let live: HashSet<&str> = all.iter().map(|t| t.id.as_str()).collect();
    if selected.iter().any(|id| !live.contains(id)) {
        return Err(AppError::validation("Có bài không còn trong catalog này"));
    }
    let tracks: Vec<&CatalogTrack> = all
        .iter()
        .filter(|t| selected.contains(t.id.as_str()))
        .collect();

    let confirmed: HashSet<&str> = req.voice_confirmed.iter().map(String::as_str).collect();
    let (mut missing_caption, mut voice_unconfirmed, mut no_tempo_key) = (vec![], vec![], vec![]);
    let mut eligible = vec![];
    for t in &tracks {
        let blank = t
            .caption
            .as_ref()
            .is_none_or(|c| c.caption.trim().is_empty());
        let voice = t.voice_warning && !confirmed.contains(t.id.as_str());
        if blank {
            missing_caption.push(build_track(t));
        } else if voice {
            voice_unconfirmed.push(build_track(t));
        } else {
            eligible.push(t.id.clone());
        }
        if t.tempo_bpm.is_none() || t.key_name.is_none() {
            no_tempo_key.push(build_track(t));
        }
    }

    let holdout_valid = (0..=5).contains(&req.holdout_count);
    let holdout_count = if holdout_valid {
        req.holdout_count as usize
    } else {
        0
    };
    let held_ids = pick_holdout(eligible.clone(), holdout_count, req.holdout_seed);
    let eligible: HashSet<String> = eligible.into_iter().collect();
    let (mut train, mut held_out) = (vec![], vec![]);
    let (mut train_seconds, mut unknown_duration) = (0.0, 0);
    let mut per_model: HashMap<String, i64> = HashMap::new();
    for t in &tracks {
        if !eligible.contains(&t.id) {
            continue;
        }
        if held_ids.contains(&t.id) {
            held_out.push(build_track(t));
            continue;
        }
        match t.duration_s {
            Some(d) => train_seconds += d.min(MAX_TRAIN_SECONDS),
            None => unknown_duration += 1,
        }
        if let Some(c) = &t.caption {
            *per_model.entry(c.model_key.clone()).or_default() += 1;
        }
        train.push(build_track(t));
    }
    let mut models = vec![];
    for (key, count) in per_model {
        let name: Option<(String,)> =
            sqlx::query_as("SELECT name FROM models WHERE key = ? LIMIT 1")
                .bind(&key)
                .fetch_optional(db)
                .await?;
        models.push(ModelCount {
            name: name.map_or_else(|| key.clone(), |n| n.0),
            model_key: key,
            count,
        });
    }
    models.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.model_key.cmp(&b.model_key))
    });

    let name = req.name.trim();
    let name_ok = !name.is_empty() && name.chars().count() <= 80;
    let name_taken = if name.is_empty() {
        false
    } else {
        let taken: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 WHERE EXISTS (SELECT 1 FROM loras WHERE status != 'archived' AND lower(name) = lower(?1)) \
                OR EXISTS (SELECT 1 FROM jobs WHERE type = 'train_lora' \
                           AND status IN ('queued', 'preparing', 'running') AND lower(title) = lower(?1))",
        )
        .bind(name)
        .fetch_optional(db)
        .await?;
        taken.is_some()
    };

    let mut errors = vec![];
    if !name_ok {
        errors.push("Tên LoRA phải có 1–80 ký tự".to_string());
    }
    if name_taken {
        errors.push("Tên LoRA đã có, hãy đặt tên khác".to_string());
    }
    if !keyword_ok(req.keyword.trim()) {
        errors.push("Từ khóa chỉ gồm chữ, số, _ hoặc -, tối đa 40 ký tự".to_string());
    }
    let prompt_len = req.test_prompt.trim().chars().count();
    if !(1..=500).contains(&prompt_len) {
        errors.push("Câu lệnh nghe thử phải có 1–500 ký tự".to_string());
    }
    if !holdout_valid {
        errors.push("Số bài giữ lại phải từ 0 đến 5".to_string());
    }
    if !missing_caption.is_empty() {
        errors.push(format!(
            "{} bài chưa có mô tả ở model đang hiện",
            missing_caption.len()
        ));
    }
    if !voice_unconfirmed.is_empty() {
        errors.push(format!(
            "{} bài có thể có giọng người, cần bỏ hoặc xác nhận",
            voice_unconfirmed.len()
        ));
    }
    if train.is_empty() {
        errors.push("Phải còn ít nhất 1 bài để train".to_string());
    }

    let mut warnings = vec![];
    if !no_tempo_key.is_empty() {
        warnings.push(format!(
            "{} bài thiếu Tempo hoặc Key, sẽ ghi N/A khi train",
            no_tempo_key.len()
        ));
    }
    warnings.push("Trong lúc train, máy render tạm không tạo nhạc được".to_string());

    let steps_per_epoch =
        (train.len() as i64 + req.gradient_accumulation - 1) / req.gradient_accumulation;
    let mut checkpoint_epochs: Vec<i64> = (1..=req.epochs)
        .filter(|e| e % req.save_every == 0)
        .collect();
    if checkpoint_epochs.last() != Some(&req.epochs) {
        checkpoint_epochs.push(req.epochs);
    }
    let eval_renders = checkpoint_epochs.len() as i64 * (1 + held_out.len() as i64) + 1;

    Ok(BuildPlan {
        ready: errors.is_empty(),
        errors,
        warnings,
        train,
        held_out,
        missing_caption,
        voice_unconfirmed,
        no_tempo_key,
        train_seconds,
        unknown_duration,
        models,
        steps_per_epoch,
        total_steps: steps_per_epoch * req.epochs,
        checkpoint_epochs,
        eval_renders,
        name_taken,
    })
}

pub async fn preview_build(
    State(state): State<AppState>,
    _trainer: Trainer,
    Path(id): Path<String>,
    Json(req): Json<BuildRequest>,
) -> AppResult<Json<BuildPlan>> {
    Ok(Json(plan_build(&state.db, &id, &req).await?))
}
