use crate::app_backup::{self, BACKUP_PREFIXES, BACKUP_SUFFIX};
use crate::domain::{AppSettings, RemoteBackupInfo};
use crate::error::{AppError, AppResult};
use crate::sync::{parse_remote_manifest_bytes, SyncManifest, SYNC_MANIFEST_FILE_NAME};
use futures_util::StreamExt;
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::{Client, Method, RequestBuilder, StatusCode};
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use url::Url;

const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PROPFIND_BYTES: u64 = 4 * 1024 * 1024;

/// 控制类请求（连接检查、列目录、读写版本指针 manifest、删除）的超时预算。
///
/// 这些请求体量都很小，超时只应让界面短暂转圈。此前它们继承客户端的 300s 总超时，
/// 一个卡住的服务器会把「测试连接」「列远端备份」变成几分钟的干等。
const CONTROL_TIMEOUT: Duration = Duration::from_secs(20);
/// 备份上传/下载的预算：包体上限 512 MiB，必须给足传输时间。
///
/// 只有这两个方法是真正的"读写大文件"，其余请求一律走 [`CONTROL_TIMEOUT`]。
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct WebDavRuntimeConfig {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub remote_path: String,
    pub accept_invalid_certs: bool,
}

pub struct WebDavClient {
    client: Client,
    config: WebDavRuntimeConfig,
    root_url: Url,
    directory_url: Url,
    /// 控制类请求预算。生产环境恒为 [`CONTROL_TIMEOUT`]，测试用它把预算压短，
    /// 以便在秒级内断言"控制类请求不继承传输用的长超时"。
    control_timeout: Duration,
}

impl WebDavClient {
    pub fn new(config: WebDavRuntimeConfig, settings: &AppSettings) -> AppResult<Self> {
        let (root_url, directory_url) = validate_and_build_urls(&config)?;
        let client = crate::http_client::build_client_with_tls(
            settings,
            TRANSFER_TIMEOUT,
            config.accept_invalid_certs,
        )?;
        Ok(Self {
            client,
            config,
            root_url,
            directory_url,
            control_timeout: CONTROL_TIMEOUT,
        })
    }

    #[cfg(test)]
    fn with_control_timeout(mut self, timeout: Duration) -> Self {
        self.control_timeout = timeout;
        self
    }

    pub async fn check_connection(&self) -> AppResult<()> {
        let status = self.propfind_status(&self.directory_url, "0").await?;
        match status {
            StatusCode::OK | StatusCode::MULTI_STATUS => Ok(()),
            StatusCode::NOT_FOUND => self.ensure_directory().await,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(AppError::new(
                "webdav_auth_failed",
                "WebDAV authentication failed",
            )),
            status => Err(http_status_error("WebDAV connection check", status)),
        }
    }

    pub async fn list_backups(&self) -> AppResult<Vec<RemoteBackupInfo>> {
        self.check_connection().await?;
        let method = webdav_method(b"PROPFIND")?;
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:"><D:prop><D:getcontentlength/><D:getlastmodified/><D:resourcetype/></D:prop></D:propfind>"#;
        let response = self
            .request(method, self.directory_url.clone())
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body)
            .send()
            .await?;
        if response.status() != StatusCode::MULTI_STATUS && !response.status().is_success() {
            return Err(http_status_error("WebDAV list", response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PROPFIND_BYTES)
        {
            return Err(AppError::new(
                "invalid_response",
                "WebDAV directory response is too large",
            ));
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if body.len() + chunk.len() > MAX_PROPFIND_BYTES as usize {
                return Err(AppError::new(
                    "invalid_response",
                    "WebDAV directory response is too large",
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let body = String::from_utf8(body).map_err(|error| {
            AppError::new(
                "invalid_response",
                format!("WebDAV directory response is not UTF-8: {error}"),
            )
        })?;
        parse_propfind_response(&body)
    }

    pub async fn upload_file(&self, file_name: &str, local_path: &Path) -> AppResult<()> {
        validate_remote_file_name(file_name)?;
        self.check_connection().await?;
        let url = append_segment(&self.directory_url, file_name)?;
        let file = tokio::fs::File::open(local_path).await?;
        let content_length = file.metadata().await?.len();
        let stream = ReaderStream::new(file);
        let response = self
            .transfer_request(Method::PUT, url)
            .header("Content-Type", "application/zip")
            .header(reqwest::header::CONTENT_LENGTH, content_length)
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await?;
        match response.status() {
            StatusCode::OK | StatusCode::CREATED | StatusCode::NO_CONTENT => Ok(()),
            status => Err(http_status_error("WebDAV upload", status)),
        }
    }

    pub async fn download_file(&self, file_name: &str, destination: &Path) -> AppResult<()> {
        validate_remote_file_name(file_name)?;
        let url = append_segment(&self.directory_url, file_name)?;
        let response = self.transfer_request(Method::GET, url).send().await?;
        if !response.status().is_success() {
            return Err(http_status_error("WebDAV download", response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_DOWNLOAD_BYTES)
        {
            return Err(AppError::new(
                "backup_invalid",
                "remote backup is too large",
            ));
        }
        let mut output = tokio::fs::File::create(destination).await?;
        let mut downloaded = 0_u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            downloaded += chunk.len() as u64;
            if downloaded > MAX_DOWNLOAD_BYTES {
                let _ = tokio::fs::remove_file(destination).await;
                return Err(AppError::new(
                    "backup_invalid",
                    "remote backup is too large",
                ));
            }
            output.write_all(&chunk).await?;
        }
        output.sync_all().await?;
        Ok(())
    }

    pub async fn upload_sync_manifest(&self, manifest: &SyncManifest) -> AppResult<()> {
        validate_remote_file_name(SYNC_MANIFEST_FILE_NAME)?;
        self.check_connection().await?;
        let url = append_segment(&self.directory_url, SYNC_MANIFEST_FILE_NAME)?;
        let body = serde_json::to_vec_pretty(manifest)?;
        let response = self
            .request(Method::PUT, url)
            .header("Content-Type", "application/json")
            .header(reqwest::header::CONTENT_LENGTH, body.len())
            .body(body)
            .send()
            .await?;
        match response.status() {
            StatusCode::OK | StatusCode::CREATED | StatusCode::NO_CONTENT => Ok(()),
            status => Err(http_status_error("WebDAV manifest upload", status)),
        }
    }

    /// 读取远端版本指针；404 表示远端还没有任何同步数据。
    pub async fn download_sync_manifest(&self) -> AppResult<Option<SyncManifest>> {
        validate_remote_file_name(SYNC_MANIFEST_FILE_NAME)?;
        let url = append_segment(&self.directory_url, SYNC_MANIFEST_FILE_NAME)?;
        let response = self.request(Method::GET, url).send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(http_status_error(
                "WebDAV manifest download",
                response.status(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > crate::sync::MAX_MANIFEST_BYTES)
        {
            return Err(AppError::new(
                "sync_manifest_invalid",
                "sync manifest is too large",
            ));
        }
        let bytes = response.bytes().await?;
        parse_remote_manifest_bytes(&bytes).map(Some)
    }

    pub async fn delete_file(&self, file_name: &str) -> AppResult<()> {
        validate_remote_file_name(file_name)?;
        let url = append_segment(&self.directory_url, file_name)?;
        let response = self.request(Method::DELETE, url).send().await?;
        match response.status() {
            StatusCode::OK | StatusCode::NO_CONTENT | StatusCode::NOT_FOUND => Ok(()),
            status => Err(http_status_error("WebDAV delete", status)),
        }
    }

    pub async fn cleanup_device_backups(
        &self,
        device_name: &str,
        max_backups: u32,
    ) -> AppResult<usize> {
        let matching = backups_to_delete(self.list_backups().await?, device_name, max_backups);
        let mut removed = 0;
        for backup in matching {
            self.delete_file(&backup.file_name).await?;
            removed += 1;
        }
        Ok(removed)
    }

    async fn ensure_directory(&self) -> AppResult<()> {
        let mut current = self.root_url.clone();
        for segment in remote_path_segments(&self.config.remote_path)? {
            current = append_segment(&current, &segment)?;
            let response = self
                .request(webdav_method(b"MKCOL")?, current.clone())
                .send()
                .await?;
            match response.status() {
                StatusCode::OK | StatusCode::CREATED | StatusCode::NO_CONTENT => {}
                StatusCode::BAD_REQUEST | StatusCode::METHOD_NOT_ALLOWED | StatusCode::CONFLICT => {
                    if !matches!(
                        self.propfind_status(&current, "0").await?,
                        StatusCode::OK | StatusCode::MULTI_STATUS
                    ) {
                        return Err(http_status_error(
                            "WebDAV directory verification",
                            response.status(),
                        ));
                    }
                }
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    return Err(AppError::new(
                        "webdav_auth_failed",
                        "WebDAV authentication failed",
                    ));
                }
                status => return Err(http_status_error("WebDAV directory creation", status)),
            }
        }
        Ok(())
    }

    async fn propfind_status(&self, url: &Url, depth: &str) -> AppResult<StatusCode> {
        let response = self
            .request(webdav_method(b"PROPFIND")?, url.clone())
            .header("Depth", depth)
            .send()
            .await?;
        Ok(response.status())
    }

    /// 控制类请求：在客户端预算之上再压一层短超时（`RequestBuilder::timeout` 从连接
    /// 建立开始计时，因此连接阶段也受它约束）。
    fn request(&self, method: Method, url: Url) -> RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.config.username, Some(&self.config.password))
            .timeout(self.control_timeout)
    }

    /// 传输类请求（备份上传/下载）：保留长预算。
    fn transfer_request(&self, method: Method, url: Url) -> RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.config.username, Some(&self.config.password))
            .timeout(TRANSFER_TIMEOUT)
    }
}

/// 远端备份排序键：先剥离已知前缀再比较。
///
/// 直接比较完整文件名会让前缀的字典序（`any-switch-backup-*` < `xiaobai-switch-backup-*`）
/// 压过时间戳，降序取前 N 时就可能误删本该保留的最新备份（manifest 当前引用的那一份），
/// 导致其它机器拉不到最新数据。剥离前缀后按时间戳自然交错比较。
fn backup_sort_key(file_name: &str) -> &str {
    app_backup::BACKUP_PREFIXES
        .iter()
        .find_map(|prefix| file_name.strip_prefix(prefix))
        .unwrap_or(file_name)
}

fn backups_to_delete(
    backups: Vec<RemoteBackupInfo>,
    device_name: &str,
    max_backups: u32,
) -> Vec<RemoteBackupInfo> {
    let mut matching = backups
        .into_iter()
        .filter(|backup| backup.device_name == device_name)
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| {
        backup_sort_key(&right.file_name).cmp(backup_sort_key(&left.file_name))
    });
    matching.into_iter().skip(max_backups as usize).collect()
}

pub fn validate_and_build_urls(config: &WebDavRuntimeConfig) -> AppResult<(Url, Url)> {
    let mut root = Url::parse(config.base_url.trim())
        .map_err(|e| AppError::new("validation_failed", format!("invalid WebDAV URL: {e}")))?;
    if !matches!(root.scheme(), "http" | "https") {
        return Err(AppError::new(
            "validation_failed",
            "WebDAV URL must use HTTP or HTTPS",
        ));
    }
    if root.host_str().is_none() {
        return Err(AppError::new(
            "validation_failed",
            "WebDAV URL must include a host",
        ));
    }
    if !root.username().is_empty() || root.password().is_some() {
        return Err(AppError::new(
            "validation_failed",
            "WebDAV URL must not contain embedded credentials",
        ));
    }
    if root.query().is_some() || root.fragment().is_some() {
        return Err(AppError::new(
            "validation_failed",
            "WebDAV URL must not contain a query or fragment",
        ));
    }
    if config.username.trim().is_empty() {
        return Err(AppError::new(
            "validation_failed",
            "WebDAV username is required",
        ));
    }
    normalize_directory_url(&mut root);
    let mut directory = root.clone();
    for segment in remote_path_segments(&config.remote_path)? {
        directory = append_segment(&directory, &segment)?;
    }
    Ok((root, directory))
}

pub fn validate_sync_interval(value: u32) -> AppResult<u32> {
    if [15, 30, 60, 120, 360, 720, 1440].contains(&value) {
        Ok(value)
    } else {
        Err(AppError::new(
            "validation_failed",
            "unsupported WebDAV sync interval",
        ))
    }
}

pub fn validate_max_backups(value: u32) -> AppResult<u32> {
    if (1..=100).contains(&value) {
        Ok(value)
    } else {
        Err(AppError::new(
            "validation_failed",
            "WebDAV retention must be between 1 and 100",
        ))
    }
}

fn remote_path_segments(path: &str) -> AppResult<Vec<String>> {
    let segments = path
        .trim()
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if segments.is_empty()
        || segments
            .iter()
            .any(|segment| matches!(segment.as_str(), "." | ".."))
    {
        return Err(AppError::new(
            "validation_failed",
            "WebDAV remote path is invalid",
        ));
    }
    Ok(segments)
}

fn normalize_directory_url(url: &mut Url) {
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
}

fn append_segment(base: &Url, segment: &str) -> AppResult<Url> {
    let mut url = base.clone();
    normalize_directory_url(&mut url);
    url.path_segments_mut()
        .map_err(|_| {
            AppError::new(
                "validation_failed",
                "WebDAV URL cannot contain path segments",
            )
        })?
        .pop_if_empty()
        .push(segment);
    if !is_file_segment(segment) {
        normalize_directory_url(&mut url);
    }
    Ok(url)
}

/// 目录段需要以 `/` 结尾；已知的数据文件（备份 zip 与同步 manifest）不需要。
fn is_file_segment(segment: &str) -> bool {
    segment.ends_with(BACKUP_SUFFIX) || segment.ends_with(".json")
}

pub(crate) fn validate_backup_file_name(file_name: &str) -> AppResult<()> {
    if BACKUP_PREFIXES
        .iter()
        .any(|prefix| file_name.starts_with(prefix))
        && file_name.ends_with(BACKUP_SUFFIX)
        && !file_name.contains('/')
        && !file_name.contains('\\')
        && !file_name.contains("..")
    {
        Ok(())
    } else {
        Err(AppError::new(
            "validation_failed",
            "invalid remote backup file name",
        ))
    }
}

/// 远端文件名白名单：备份 zip 或同步版本指针 manifest。
fn validate_remote_file_name(file_name: &str) -> AppResult<()> {
    if file_name == SYNC_MANIFEST_FILE_NAME {
        return Ok(());
    }
    validate_backup_file_name(file_name)
}

fn webdav_method(value: &[u8]) -> AppResult<Method> {
    Method::from_bytes(value)
        .map_err(|e| AppError::new("internal", format!("invalid WebDAV method: {e}")))
}

fn http_status_error(action: &str, status: StatusCode) -> AppError {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        AppError::new("webdav_auth_failed", "WebDAV authentication failed")
    } else {
        AppError::new("network", format!("{action} failed with HTTP {status}"))
    }
}

#[derive(Default)]
struct ParsedResponse {
    href: String,
    size: u64,
    last_modified: String,
    collection: bool,
}

fn parse_propfind_response(xml: &str) -> AppResult<Vec<RemoteBackupInfo>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut current: Option<ParsedResponse> = None;
    let mut current_tag = Vec::new();
    let mut backups = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let name = event.local_name().as_ref().to_ascii_lowercase();
                if name == b"response" {
                    current = Some(ParsedResponse::default());
                } else if current.is_some() {
                    if name == b"collection" {
                        current.as_mut().unwrap().collection = true;
                    }
                    current_tag = name;
                }
            }
            Ok(Event::Empty(event)) => {
                if current.is_some()
                    && event
                        .local_name()
                        .as_ref()
                        .eq_ignore_ascii_case(b"collection")
                {
                    current.as_mut().unwrap().collection = true;
                }
            }
            Ok(Event::Text(text)) => {
                if let Some(response) = current.as_mut() {
                    let value = text
                        .decode()
                        .map_err(|e| {
                            AppError::new("invalid_response", format!("invalid WebDAV XML: {e}"))
                        })?
                        .trim()
                        .to_string();
                    match current_tag.as_slice() {
                        b"href" if response.href.is_empty() => response.href = value,
                        b"getcontentlength" if response.size == 0 => {
                            response.size = value.parse().unwrap_or(0)
                        }
                        b"getlastmodified" if response.last_modified.is_empty() => {
                            response.last_modified = value
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(event)) => {
                let name = event.local_name().as_ref().to_ascii_lowercase();
                if name == b"response" {
                    if let Some(response) = current.take() {
                        if !response.collection {
                            let file_name = response.href.rsplit('/').next().unwrap_or("");
                            if validate_backup_file_name(file_name).is_ok() {
                                backups.push(RemoteBackupInfo {
                                    file_name: file_name.to_string(),
                                    size: response.size,
                                    last_modified: response.last_modified,
                                    device_name: app_backup::parse_device_from_filename(file_name),
                                });
                            }
                        }
                    }
                }
                current_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(AppError::new(
                    "invalid_response",
                    format!("invalid WebDAV XML: {error}"),
                ))
            }
            _ => {}
        }
    }
    backups.sort_by(|left, right| {
        backup_sort_key(&right.file_name).cmp(backup_sort_key(&left.file_name))
    });
    Ok(backups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn config() -> WebDavRuntimeConfig {
        WebDavRuntimeConfig {
            base_url: "https://dav.example.com/root/".into(),
            username: "alice".into(),
            password: "secret".into(),
            remote_path: "同步/备份".into(),
            accept_invalid_certs: false,
        }
    }

    #[test]
    fn builds_encoded_directory_url_without_embedded_credentials() {
        let (_, directory) = validate_and_build_urls(&config()).unwrap();
        assert_eq!(
            directory.as_str(),
            "https://dav.example.com/root/%E5%90%8C%E6%AD%A5/%E5%A4%87%E4%BB%BD/"
        );
        let mut invalid = config();
        invalid.base_url = "https://alice:secret@dav.example.com/".into();
        assert!(validate_and_build_urls(&invalid).is_err());
    }

    #[test]
    fn parses_namespaced_propfind_and_filters_other_files() {
        // 新旧前缀混存：更旧的一份是当前前缀、更新的一份是旧前缀，专门覆盖
        // “按完整文件名排序会把旧前缀排在前面” 的回归（naive 排序会得到 [42, 84]）。
        let xml = r#"<D:multistatus xmlns:D="DAV:">
          <D:response><D:href>/root/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>
          <D:response><D:href>/root/xiaobai-switch-backup-20260827_120000.mac.12345678.zip</D:href><D:propstat><D:prop><D:getcontentlength>42</D:getcontentlength><D:getlastmodified>today</D:getlastmodified></D:prop></D:propstat></D:response>
          <d:response xmlns:d="DAV:"><d:href>/root/any-switch-backup-20260828_120000.mac.87654321.zip</d:href><d:propstat><d:prop><d:getcontentlength>84</d:getcontentlength><d:getlastmodified>tomorrow</d:getlastmodified></d:prop></d:propstat></d:response>
          <D:response><D:href>/root/notes.txt</D:href></D:response>
        </D:multistatus>"#;
        let backups = parse_propfind_response(xml).unwrap();
        assert_eq!(backups.len(), 2);
        // 20260828（旧前缀但更新）必须排在 20260827（当前前缀）之前。
        assert_eq!(backups[0].size, 84);
        assert_eq!(backups[0].device_name, "mac");
        assert_eq!(backups[1].size, 42);
    }

    #[test]
    fn accepts_legacy_and_current_remote_backup_file_names() {
        assert!(validate_backup_file_name("xiaobai-switch-backup-20260827_120000.mac.12345678.zip")
            .is_ok());
        assert!(
            validate_backup_file_name("any-switch-backup-20260827_120000.mac.12345678.zip").is_ok(),
            "legacy remote backups must stay readable"
        );
        assert!(validate_remote_file_name(SYNC_MANIFEST_FILE_NAME).is_ok());
        assert!(validate_backup_file_name("notes.txt").is_err());
        assert!(validate_backup_file_name("xiaobai-switch-backup-../../etc/passwd").is_err());
    }

    #[test]
    fn interval_and_retention_are_strictly_validated() {
        assert_eq!(validate_sync_interval(60).unwrap(), 60);
        assert!(validate_sync_interval(61).is_err());
        assert_eq!(validate_max_backups(100).unwrap(), 100);
        assert!(validate_max_backups(0).is_err());
    }

    async fn mock_webdav_server(
        statuses: Vec<&'static str>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for status in statuses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 1024];
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&bytes).to_string());
                let response =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        (format!("http://{address}/"), task)
    }

    fn local_config(base_url: String) -> WebDavRuntimeConfig {
        WebDavRuntimeConfig {
            base_url,
            username: "alice".into(),
            password: "secret".into(),
            remote_path: "backups".into(),
            accept_invalid_certs: false,
        }
    }

    /// 超时分层：控制类请求（列目录、读写版本指针等）走自己的短预算，
    /// 不继承传输用的长预算——否则一个卡住的服务器会让界面干等几分钟。
    #[tokio::test]
    async fn control_requests_use_their_own_short_budget() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        // 接受连接后一个字都不回：模拟卡死的服务器。
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(socket);
        });
        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        let client = WebDavClient::new(local_config(format!("http://{address}/")), &settings)
            .unwrap()
            .with_control_timeout(Duration::from_millis(500));

        let started = std::time::Instant::now();
        let error = client.check_connection().await.unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "control requests must not inherit the transfer timeout"
        );
        assert_eq!(serde_json::to_value(error).unwrap()["code"], "timeout");
        assert!(
            TRANSFER_TIMEOUT > CONTROL_TIMEOUT,
            "transfers keep the longer budget, control requests the shorter one"
        );
        server.abort();
    }

    #[tokio::test]
    async fn reports_authentication_failure_without_exposing_credentials() {
        let (base_url, server) = mock_webdav_server(vec!["401 Unauthorized"]).await;
        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        let client = WebDavClient::new(local_config(base_url), &settings).unwrap();
        let error = client.check_connection().await.unwrap_err();
        let serialized = serde_json::to_value(error).unwrap();
        assert_eq!(serialized["code"], "webdav_auth_failed");
        assert!(!serialized.to_string().contains("secret"));
        let requests = server.await.unwrap();
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("authorization: basic"));
    }

    #[tokio::test]
    async fn creates_a_missing_remote_directory() {
        let (base_url, server) = mock_webdav_server(vec!["404 Not Found", "201 Created"]).await;
        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        let client = WebDavClient::new(local_config(base_url), &settings).unwrap();
        client.check_connection().await.unwrap();
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("PROPFIND /backups/ "));
        assert!(requests[1].starts_with("MKCOL /backups/ "));
    }

    #[tokio::test]
    async fn verifies_an_ambiguous_mkcol_response_with_propfind() {
        let (base_url, server) = mock_webdav_server(vec![
            "404 Not Found",
            "405 Method Not Allowed",
            "207 Multi-Status",
        ])
        .await;
        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        let client = WebDavClient::new(local_config(base_url), &settings).unwrap();
        client.check_connection().await.unwrap();
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("PROPFIND /backups/ "));
        assert!(requests[1].starts_with("MKCOL /backups/ "));
        assert!(requests[2].starts_with("PROPFIND /backups/ "));
    }

    fn remote_backup(file_name: &str, device_name: &str) -> RemoteBackupInfo {
        RemoteBackupInfo {
            file_name: file_name.into(),
            size: 1,
            last_modified: String::new(),
            device_name: device_name.into(),
        }
    }

    fn kept_file_names(all: &[RemoteBackupInfo], removed: &[RemoteBackupInfo]) -> Vec<String> {
        all.iter()
            .filter(|backup| {
                !removed
                    .iter()
                    .any(|item| item.file_name == backup.file_name)
            })
            .map(|backup| backup.file_name.clone())
            .collect()
    }

    #[test]
    fn remote_retention_only_prunes_old_backups_for_the_current_device() {
        // 混合前缀：naive 的“按完整文件名降序”会先删掉最新的 20260803（旧前缀字典序靠前），
        // 正确行为是删除最旧的 20260801。
        let all = vec![
            remote_backup("xiaobai-switch-backup-20260801_000000.mac.1.zip", "mac"),
            remote_backup("xiaobai-switch-backup-20260802_000000.mac.2.zip", "mac"),
            remote_backup("any-switch-backup-20260803_000000.mac.3.zip", "mac"),
            remote_backup("xiaobai-switch-backup-20260801_000000.pc.1.zip", "pc"),
        ];
        let removed = backups_to_delete(all.clone(), "mac", 2);
        assert_eq!(removed.len(), 1);
        assert!(removed[0].file_name.contains("20260801"));
        assert_eq!(removed[0].device_name, "mac");
        assert!(
            !removed
                .iter()
                .any(|item| item.file_name.starts_with("any-switch-backup-")),
            "the newest cross-prefix bundle must survive retention"
        );
    }

    #[test]
    fn remote_retention_keeps_newest_bundles_across_prefixes() {
        // retention=1：旧 1 + 新 1，必须保留刚上传的新包（manifest 当前引用）。
        let all = vec![
            remote_backup(
                "any-switch-backup-20260801_000000.mac.old00001.zip",
                "mac",
            ),
            remote_backup(
                "xiaobai-switch-backup-20260802_000000.mac.new00001.zip",
                "mac",
            ),
        ];
        let removed = backups_to_delete(all.clone(), "mac", 1);

        assert_eq!(removed.len(), 1);
        assert!(removed[0].file_name.starts_with("any-switch-backup-"));
        let kept = kept_file_names(&all, &removed);
        assert_eq!(
            kept,
            vec!["xiaobai-switch-backup-20260802_000000.mac.new00001.zip".to_string()],
            "the newest bundle referenced by the remote manifest must be kept"
        );
    }

    #[test]
    fn remote_retention_keeps_the_latest_bundle_the_manifest_points_to() {
        let mut all = Vec::new();
        for index in 1..=3 {
            all.push(remote_backup(
                &format!("any-switch-backup-2026080{index}_000000.mac.old{index}.zip"),
                "mac",
            ));
            all.push(remote_backup(
                &format!("xiaobai-switch-backup-2026090{index}_000000.mac.new{index}.zip"),
                "mac",
            ));
        }
        let latest = all
            .iter()
            .max_by(|left, right| {
                backup_sort_key(&left.file_name).cmp(backup_sort_key(&right.file_name))
            })
            .unwrap()
            .file_name
            .clone();

        let removed = backups_to_delete(all.clone(), "mac", 3);
        let kept = kept_file_names(&all, &removed);

        assert_eq!(kept.len(), 3);
        assert!(
            kept.contains(&latest),
            "retention must never delete the latest bundle: {latest}"
        );
        assert!(
            removed
                .iter()
                .all(|item| item.file_name.starts_with("any-switch-backup-")),
            "with 3 old + 3 new and retention=3 the three legacy bundles are pruned"
        );
        assert!(kept.iter().all(|name| name.starts_with("xiaobai-switch-backup-")));
    }
}
