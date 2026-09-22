use crate::domain::SiteQuota;
use crate::error::{AppError, AppResult};
use crate::quota_probe::NewApiAccessProbe;
use crate::repo;
use crate::state::AppState;
use tauri::State;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestNewApiAccessInput {
    pub base_url: String,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub site_id: Option<String>,
}

/// 站点编辑里的「测试」按钮：验证访问令牌 + 用户 ID 能否查到账户余额。
#[tauri::command]
pub async fn test_newapi_access(
    state: State<'_, AppState>,
    input: TestNewApiAccessInput,
) -> AppResult<NewApiAccessProbe> {
    let token = input
        .access_token
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let user_id = input
        .user_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let (token, user_id, base_url) = match (token, user_id) {
        (Some(token), Some(user_id)) => (token, user_id, input.base_url),
        _ => {
            let site_id = input.site_id.clone().ok_or_else(|| {
                AppError::new("validation_failed", "access token and user id are required")
            })?;
            let stored = state.db.with_conn(|c| {
                let site = repo::site::get_site(c, &site_id)?;
                let token = repo::site::get_site_newapi_token(c, &state.crypto, &site_id)?;
                let user_id = site.newapi_user_id.clone().ok_or_else(|| {
                    AppError::new("validation_failed", "user id is required")
                })?;
                Ok((token, user_id, site.base_url))
            })?;
            let base_url = if input.base_url.trim().is_empty() {
                stored.2
            } else {
                input.base_url
            };
            (stored.0, stored.1, base_url)
        }
    };
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    crate::quota_probe::test_newapi_access(&base_url, &token, &user_id, &settings).await
}

#[tauri::command]
pub async fn probe_site_quota(state: State<'_, AppState>, site_id: String) -> AppResult<SiteQuota> {
    probe_quota_for(&state, &site_id).await
}

/// 探测单个站点的额度。
///
/// 从命令层抽出来，让内部调用方（如批量刷新）能直接拿到 `&AppState` 复用同一份逻辑。
pub(crate) async fn probe_quota_for(state: &AppState, site_id: &str) -> AppResult<SiteQuota> {
    let (site, api_key, newapi, settings) = state.db.with_conn(|c| {
        let site = repo::site::get_site(c, site_id)?;
        let key = repo::site_api_key::get_active(c, site_id)?;
        let secret = repo::site_api_key::decrypt(&state.crypto, &key)?;
        let newapi = match (&site.newapi_access_token_encrypted, &site.newapi_user_id) {
            (Some(token), Some(user_id)) if !token.is_empty() && !user_id.is_empty() => {
                Some((state.crypto.decrypt(token)?, user_id.clone()))
            }
            _ => None,
        };
        let settings = repo::settings::get_settings(c)?;
        Ok((site, secret, newapi, settings))
    })?;
    if api_key.trim().is_empty() && !crate::quota_probe::allows_empty_key_probe(&site.base_url) {
        return Ok(crate::quota_probe::empty_key_result());
    }
    crate::quota_probe::probe_quota(
        &site,
        &api_key,
        &settings,
        newapi.as_ref().map(|(token, user_id)| (token.as_str(), user_id.as_str())),
    )
    .await
}
