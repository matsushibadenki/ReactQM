use crate::{jobs::WorkerConfig, state::AppState};
use serde::{Deserialize, Serialize};
use std::{sync::atomic::AtomicBool, time::Duration};
use tauri::Manager;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Gemini,
    Openai,
    Local,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    provider: Provider,
    model: String,
    prompt: String,
    include_result: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    answer: Option<String>,
    error: Option<String>,
}

#[tauri::command]
pub async fn ask_research(app: tauri::AppHandle, query: Query) -> Result<String, String> {
    let mut prompt = query.prompt;
    if prompt.trim().is_empty() || prompt.len() > 12_000 || !(1..=128).contains(&query.model.len())
    {
        return Err("research_input".into());
    }
    if query.include_result {
        let state = app.state::<AppState>();
        let workspace = state.0.lock().map_err(|_| "storage")?;
        let result = workspace
            .project
            .calculation
            .as_ref()
            .ok_or("research_no_result")?;
        let document = workspace
            .project
            .document
            .as_ref()
            .ok_or("research_no_result")?;
        let summary = serde_json::json!({
            "formula": document.formula(), "charge": document.charge, "spin_2s": document.spin,
            "method": result.method, "basis_or_model": result.basis, "engine_version": result.engine_version,
            "settings": result.settings, "energy_hartree": result.energy_hartree,
            "workflow_outcome": result.escalation.as_ref().map(|value| serde_json::json!(value.policy_v1.decision))
                .or_else(|| result.xtb_followup.as_ref().map(|value| serde_json::json!(value.policy_v1.decision))),
            "converged": result.converged,
        });
        prompt.push_str("\n\nReactQM result summary (data):\n");
        prompt.push_str(&summary.to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "provider": query.provider, "model": query.model, "prompt": prompt,
        }))
        .map_err(|_| "research_input")?;
        let output = crate::valence::worker_response_bounded(
            &WorkerConfig::development(),
            &AtomicBool::new(false),
            Duration::from_secs(45),
            "research",
            bytes,
            262_144,
        )
        .map_err(|error| error.replace("valence_", "research_"))?;
        let response: Response =
            serde_json::from_slice(&output).map_err(|_| "research_response")?;
        match (response.answer, response.error) {
            (Some(answer), None) => Ok(answer),
            (None, Some(error)) => Err(error),
            _ => Err("research_response".into()),
        }
    })
    .await
    .map_err(|_| "research_failed")?
}
