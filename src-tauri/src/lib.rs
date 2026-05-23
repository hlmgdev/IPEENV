use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::{
    collections::HashMap,
    env, fs,
    fs::File,
    io::{Read, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, State, WindowEvent};
use tauri::tray::{TrayIconBuilder, MouseButton, MouseButtonState};
use tauri::menu::{Menu, MenuItem};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const PACKAGE_CATALOG_PATH: &str = "config/component-catalog.conf";
const SITE_TEMPLATE_CATALOG_PATH: &str = "config/project-templates.conf";
const LEGACY_PACKAGE_CATALOG_PATH: &str = "packages.conf";
const LEGACY_SITE_TEMPLATE_CATALOG_PATH: &str = "dependencias/sites.conf";

#[derive(Default)]
struct ProcessState {
    children: Mutex<HashMap<String, Child>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    root_dir: PathBuf,
    http_port: u16,
    https_port: u16,
    mysql_port: u16,
    php_version_hint: String,
    #[serde(default = "default_https_enabled")]
    https_enabled: bool,
    #[serde(default = "default_enabled_services")]
    enabled_services: Vec<String>,
}

impl AppConfig {
    fn default_for(root_dir: PathBuf) -> Self {
        Self {
            root_dir,
            http_port: 80,
            https_port: 443,
            mysql_port: 3306,
            php_version_hint: "PHP empacotado".to_string(),
            https_enabled: true,
            enabled_services: default_enabled_services(),
        }
    }
}

fn default_https_enabled() -> bool {
    true
}

fn default_enabled_services() -> Vec<String> {
    vec!["apache".to_string(), "mysql".to_string()]
}

#[derive(Debug, Clone, Serialize)]
struct EnvironmentInfo {
    app_version: String,
    root_dir: String,
    http_port: u16,
    https_port: u16,
    mysql_port: u16,
    https_enabled: bool,
    folders: Vec<FolderInfo>,
    services: Vec<ServiceInfo>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
struct FolderInfo {
    name: String,
    path: String,
    exists: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ServiceInfo {
    id: String,
    name: String,
    version: String,
    port: Option<u16>,
    status: String,
    pid: Option<u32>,
    executable: String,
    available: bool,
    port_available: Option<bool>,
    last_message: String,
    enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
struct Diagnostic {
    level: String,
    title: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectInfo {
    name: String,
    path: String,
    vhost_path: String,
    http_url: String,
    https_url: String,
    domain: String,
    php_version: Option<String>,
    php_cgi_port: Option<u16>,
    ssl_enabled: bool,
    host_configured: bool,
    framework: String,
    modified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectConfig {
    php_version: Option<String>,
    template: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CreateProjectRequest {
    name: String,
    template: Option<String>,
    php_version: Option<String>,
    add_host: bool,
    enable_ssl: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct PortUpdate {
    http_port: u16,
    https_port: u16,
    mysql_port: u16,
}

#[derive(Debug, Clone, Deserialize)]
struct HttpsUpdate {
    enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ActionResult {
    ok: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    params: HashMap<String, String>,
}

impl ActionResult {
    fn message(ok: bool, message: impl Into<String>) -> Self {
        Self {
            ok,
            message: message.into(),
            code: None,
            params: HashMap::new(),
        }
    }

    fn coded(
        ok: bool,
        code: impl Into<String>,
        message: impl Into<String>,
        params: impl IntoIterator<Item = (&'static str, String)>,
    ) -> Self {
        Self {
            ok,
            message: message.into(),
            code: Some(code.into()),
            params: params
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct LogBundle {
    kind: String,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct PackageEntry {
    name: String,
    url: String,
    category: String,
    preferred: bool,
    install_dir: String,
    installed: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SiteTemplate {
    name: String,
    framework: String,
    version: String,
    source: String,
    category: String,
    php_min: Option<String>,
    php_max: Option<String>,
    preferred: bool,
}

#[derive(Debug, Clone, Serialize)]
struct PhpOption {
    version: String,
    label: String,
    installed: bool,
    installable: bool,
}

#[derive(Debug, Clone, Serialize)]
struct PhpRuntimeInfo {
    version: String,
    name: String,
    path: String,
    ini_path: String,
    extension_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct PhpExtensionInfo {
    name: String,
    dll: String,
    enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ToolInfo {
    id: String,
    name: String,
    kind: String,
    source_path: String,
    install_path: String,
    installed: bool,
    available_source: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ProjectProgress {
    project: String,
    step: String,
    percent: u8,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
struct InstallProgress {
    item: String,
    kind: String,
    step: String,
    percent: u8,
    status: String,
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let root = app_root(app.handle())?;
            ensure_environment(&root)?;
            mirror_bundled_resources(app.handle(), &root)?;
            migrate_flat_runtime_dirs(&root)?;
            warn_missing_legacy_php_runtime_dependency(&root)?;
            seed_package_catalog(app.handle(), &root)?;
            seed_site_template_catalog(app.handle(), &root)?;
            install_bundled_portable_tools(app.handle(), &root)?;
            cleanup_bundled_resource_dir(app.handle(), &root)?;
            append_app_log(&root, "Ipeenv initialized")?;

            let quit_i = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
            let start_all = MenuItem::with_id(app, "start_all", "Iniciar todos os serviços", true, None::<&str>)?;
            let stop_all = MenuItem::with_id(app, "stop_all", "Parar todos os serviços", true, None::<&str>)?;
            let start_apache = MenuItem::with_id(app, "start_apache", "Iniciar Apache", true, None::<&str>)?;
            let stop_apache = MenuItem::with_id(app, "stop_apache", "Parar Apache", true, None::<&str>)?;
            let start_mysql = MenuItem::with_id(app, "start_mysql", "Iniciar MySQL", true, None::<&str>)?;
            let stop_mysql = MenuItem::with_id(app, "stop_mysql", "Parar MySQL", true, None::<&str>)?;
            
            let menu = Menu::with_items(app, &[
                &start_all, &stop_all, 
                &tauri::menu::PredefinedMenuItem::separator(app)?,
                &start_apache, &stop_apache,
                &tauri::menu::PredefinedMenuItem::separator(app)?,
                &start_mysql, &stop_mysql,
                &tauri::menu::PredefinedMenuItem::separator(app)?,
                &quit_i
            ])?;
            
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "quit" => {
                            #[cfg(target_os = "windows")]
                            {
                                let _ = Command::new("taskkill").args(["/F", "/IM", "httpd.exe"]).creation_flags(CREATE_NO_WINDOW).output();
                                let _ = Command::new("taskkill").args(["/F", "/IM", "mysqld.exe"]).creation_flags(CREATE_NO_WINDOW).output();
                                let _ = Command::new("taskkill").args(["/F", "/IM", "php-cgi.exe"]).creation_flags(CREATE_NO_WINDOW).output();
                            }
                            app.exit(0);
                        }
                        action => {
                            let _ = app.emit("tray-action", action);
                        }
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click { button, button_state, .. } = event {
                        if button == MouseButton::Left && button_state == MouseButtonState::Up {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                let _ = window.hide();
                api.prevent_close();
            }
            _ => {}
        })
        .manage(ProcessState::default())
        .invoke_handler(tauri::generate_handler![
            quit,
            get_environment_info,
            list_projects,
            create_project,
            enable_service,
            disable_service,
            start_service,
            stop_service,
            restart_service,
            update_ports,
            update_https,
            read_logs,
            open_path,
            open_www_folder,
            open_vhosts_folder,
            open_url,
            open_project,
            open_vhost_file,
            list_packages,
            install_package,
            open_packages_config,
            list_site_templates,
            list_php_options,
            list_php_runtimes,
            list_php_extensions,
            set_php_extension,
            open_php_ini,
            open_sites_config,
            list_local_tools,
            install_local_tool,
            launch_tool,
            add_host,
            remove_host,
            enable_ssl,
            generate_apache_config,
            allow_firewall,
            open_hosts_file
        ])
        .run(tauri::generate_context!())
        .expect("erro ao executar o Ipeenv");
}

fn warn_missing_legacy_php_runtime_dependency(root: &Path) -> Result<(), String> {
    let deps = workspace_root().join("dependencias");
    let deps_local = root.join("dependencias");
    let has_vc2012 = deps.join("vcredist_x64_2012.exe").exists()
        || deps.join("vcredist_x64.exe").exists()
        || deps_local.join("vcredist_x64_2012.exe").exists()
        || deps_local.join("vcredist_x64.exe").exists();
    let has_vc14plus = deps.join("VC_redist.x64.exe").exists()
        || deps.join("vcredist_x64_2015_2022.exe").exists()
        || deps.join("vcredist_x64_2019.exe").exists()
        || deps_local.join("VC_redist.x64.exe").exists()
        || deps_local.join("vcredist_x64_2015_2022.exe").exists()
        || deps_local.join("vcredist_x64_2019.exe").exists();
    if !has_vc2012 {
        append_app_log(
            root,
            "Recommended dependency missing: place dependencias/vcredist_x64_2012.exe to support legacy PHP (for example 5.6/VC11) without relying on winget/internet.",
        )?;
    }
    if !has_vc14plus {
        append_app_log(
            root,
            "Recommended dependency missing: place dependencias/VC_redist.x64.exe to support PHP VC14+/VS16/VS17.",
        )?;
    }
    Ok(())
}

#[tauri::command]
fn quit(app: tauri::AppHandle) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("taskkill").args(["/F", "/IM", "httpd.exe"]).creation_flags(CREATE_NO_WINDOW).output();
        let _ = Command::new("taskkill").args(["/F", "/IM", "mysqld.exe"]).creation_flags(CREATE_NO_WINDOW).output();
        let _ = Command::new("taskkill").args(["/F", "/IM", "php-cgi.exe"]).creation_flags(CREATE_NO_WINDOW).output();
    }
    app.exit(0);
}

#[tauri::command]
fn get_environment_info(
    app: tauri::AppHandle,
    state: State<ProcessState>,
) -> Result<EnvironmentInfo, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let cfg = read_config(&root)?;
    let services = service_catalog(&cfg, &state);
    let mut diagnostics = Vec::new();

    for service in &services {
        if !service.available {
            diagnostics.push(Diagnostic {
                level: "warn".to_string(),
                title: format!("{} não encontrado", service.name),
                message: format!("Coloque o executavel esperado em {}", service.executable),
            });
        }
        if service.port_available == Some(false) && service.status != "running" {
            diagnostics.push(Diagnostic {
                level: "error".to_string(),
                title: format!("Porta ocupada: {}", service.port.unwrap_or_default()),
                message: format!(
                    "{} não pode iniciar enquanto essa porta estiver em uso.",
                    service.name
                ),
            });
        }
    }

    Ok(EnvironmentInfo {
        app_version: APP_VERSION.to_string(),
        root_dir: display_path(&root),
        http_port: cfg.http_port,
        https_port: cfg.https_port,
        mysql_port: cfg.mysql_port,
        https_enabled: cfg.https_enabled,
        folders: required_folders()
            .into_iter()
            .map(|name| {
                let path = root.join(name);
                FolderInfo {
                    name: name.to_string(),
                    path: display_path(&path),
                    exists: path.exists(),
                }
            })
            .collect(),
        services,
        diagnostics,
    })
}

#[tauri::command]
fn list_projects(app: tauri::AppHandle) -> Result<Vec<ProjectInfo>, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    list_projects_from_root(&root)
}

fn list_projects_from_root(root: &Path) -> Result<Vec<ProjectInfo>, String> {
    let hosts = read_hosts_file().unwrap_or_default();
    let ssl_root = root.join("etc").join("ssl").join("certs");
    let www = root.join("www");
    let mut projects = Vec::new();

    for entry in fs::read_dir(&www).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let domain = format!("{}.test", slugify(&name));
        let project_config = read_project_config(&path).unwrap_or(ProjectConfig {
            php_version: None,
            template: None,
        });
        let metadata = entry.metadata().ok();
        let modified_at = metadata
            .and_then(|m| m.modified().ok())
            .map(|t| {
                DateTime::<Local>::from(t)
                    .format("%d/%m/%Y %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| "-".to_string());

        projects.push(ProjectInfo {
            name: name.clone(),
            path: display_path(&path),
            vhost_path: display_path(
                &root
                    .join("etc")
                    .join("apache")
                    .join("vhosts")
                    .join(format!("{}.conf", domain)),
            ),
            http_url: format!("http://{}", domain),
            https_url: format!("https://{}", domain),
            php_cgi_port: project_config.php_version.as_deref().map(php_cgi_port),
            php_version: project_config.php_version,
            host_configured: hosts.contains(&domain),
            ssl_enabled: ssl_root.join(format!("{}.crt", domain)).exists(),
            framework: detect_framework(&path),
            modified_at,
            domain,
        });
    }

    projects.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(projects)
}

#[tauri::command]
fn create_project(
    app: tauri::AppHandle,
    state: State<ProcessState>,
    request: CreateProjectRequest,
) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    seed_site_template_catalog(&app, &root)?;
    let name = slugify(&request.name);
    if name.is_empty() {
        return Err("Informe um nome de projeto valido.".to_string());
    }
    emit_project_progress(&app, &name, "Validando projeto", 5, "running");

    let project_dir = root.join("www").join(&name);
    if project_dir.exists() {
        emit_project_progress(&app, &name, "Projeto ja existe", 100, "error");
        return Err(format!("O projeto '{}' ja existe.", name));
    }

    let selected_php = request
        .php_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let selected_template = request
        .template
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| load_template_by_name(&root, value));

    let mut effective_php = selected_php.map(str::to_string);
    let mut php_note = None::<String>;
    if let Some(template) = &selected_template {
        if let Some(selected) = selected_php {
            if let Some(minimum) = template.php_min.as_deref() {
                if !version_at_least(selected, minimum) {
                    return Err(format!(
                        "PHP {} não atende o template selecionado (mínimo requerido: {}).",
                        selected, minimum
                    ));
                }
            }
            if let Some(maximum) = template.php_max.as_deref() {
                if !version_at_most(selected, maximum) {
                    return Err(format!(
                        "PHP {} não atende o template selecionado (máximo permitido: {}).",
                        selected, maximum
                    ));
                }
            }
        } else {
            let fallback = resolve_compatible_php_version_range(
                &root,
                template.php_min.as_deref(),
                template.php_max.as_deref(),
            )
            .ok_or_else(|| {
                "Não foi possível determinar um PHP compatível para o template.".to_string()
            })?;
            php_note = Some(format!(
                "Nenhum PHP selecionado; Ipeenv usou PHP {} compatível.",
                fallback
            ));
            effective_php = Some(fallback);
        }
    }
    if let Some(version) = effective_php.as_deref() {
        emit_project_progress(
            &app,
            &name,
            &format!("Verificando PHP {}", version),
            15,
            "running",
        );
        ensure_php_version_installed(&app, &root, version)?;
    }

    emit_project_progress(&app, &name, "Criando arquivos do projeto", 35, "running");
    if let Err(err) = create_project_from_template(
        &app,
        &root,
        &project_dir,
        &name,
        request.template.as_deref(),
        effective_php.as_deref(),
    ) {
        if project_dir.exists()
            && fs::read_dir(&project_dir)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false)
        {
            let _ = fs::remove_dir(&project_dir);
        }
        emit_project_progress(&app, &name, "Falha ao criar projeto", 100, "error");
        return Err(err);
    }
    write_project_config(
        &project_dir,
        &ProjectConfig {
            php_version: effective_php.clone(),
            template: request.template.clone(),
        },
    )?;

    let domain = format!("{}.test", name);
    let mut notes = vec![format!("Projeto criado em {}", display_path(&project_dir))];
    if let Some(note) = php_note {
        notes.push(note);
    }

    if request.enable_ssl {
        emit_project_progress(&app, &name, "Preparando SSL local", 70, "running");
        if read_config(&root)
            .map(|cfg| cfg.https_enabled)
            .unwrap_or(true)
        {
            enable_ssl(app.clone(), domain.clone())?;
            notes.push("arquivos SSL locais preparados".to_string());
        } else {
            notes.push("HTTPS local está desabilitado nas preferências".to_string());
        }
    }
    if request.add_host {
        emit_project_progress(&app, &name, "Atualizando hosts", 82, "running");
        match add_host(app.clone(), domain.clone()) {
            Ok(result) => notes.push(result.message),
            Err(err) => notes.push(format!("hosts não atualizado: {}", err)),
        }
    }

    emit_project_progress(&app, &name, "Regenerando configurações", 92, "running");
    let _ = generate_apache_config(app.clone());
    emit_project_progress(&app, &name, "Iniciando Apache", 96, "running");
    if let Some(spec) = service_spec(&read_config(&root)?, "apache") {
        if spec.executable.exists() {
            let result = if service_is_running(&state, "apache", spec.port)? {
                restart_service_internal(app.clone(), &state, "apache")?
            } else {
                start_service_internal(app.clone(), &state, "apache")?
            };
            if !result.ok {
                emit_project_progress(&app, &name, "Apache não iniciou", 96, "error");
                return Err(result.message);
            }
            notes.push(result.message);
        } else {
            notes.push("Apache ainda não está instalado".to_string());
        }
    }
    append_app_log(&root, &format!("Project '{}' created", name))?;
    emit_project_progress(&app, &name, "Projeto criado", 100, "done");

    Ok(ActionResult::message(true, notes.join("; ")))
}

#[tauri::command]
fn enable_service(app: tauri::AppHandle, service: String) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let mut cfg = read_config(&root)?;
    if !cfg.enabled_services.contains(&service) {
        cfg.enabled_services.push(service.clone());
        write_config(&root, &cfg)?;
    }
    Ok(ActionResult::message(true, format!("{} habilitado", service)))
}

#[tauri::command]
fn disable_service(app: tauri::AppHandle, service: String) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let mut cfg = read_config(&root)?;
    if let Some(pos) = cfg.enabled_services.iter().position(|x| *x == service) {
        cfg.enabled_services.remove(pos);
        write_config(&root, &cfg)?;
    }
    Ok(ActionResult::message(true, format!("{} desabilitado", service)))
}

#[tauri::command]
fn start_service(
    app: tauri::AppHandle,
    state: State<ProcessState>,
    service: String,
) -> Result<ActionResult, String> {
    start_service_internal(app, &state, &service)
}

fn start_service_internal(
    app: tauri::AppHandle,
    state: &State<ProcessState>,
    service: &str,
) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    if service == "apache" {
        generate_apache_config(app.clone())?;
    }
    let cfg = read_config(&root)?;
    let spec = service_spec(&cfg, &service).ok_or_else(|| "Servico desconhecido.".to_string())?;

    if service != "apache"
        && state
            .children
            .lock()
            .map_err(|e| e.to_string())?
            .contains_key(&spec.id)
    {
        return Ok(ActionResult::coded(
            true,
            "service.alreadyRunning",
            format!("{} já está em execução.", spec.name),
            [("service", spec.name.to_string())],
        ));
    }
    if !spec.executable.exists() {
        append_app_log(
            &root,
            &format!("{} not started: missing binary", spec.name),
        )?;
        return Ok(ActionResult::coded(
            false,
            "service.missingBinary",
            format!(
                "{} não encontrado em {}.",
                spec.name,
                display_path(&spec.executable)
            ),
            [
                ("service", spec.name.to_string()),
                ("path", display_path(&spec.executable)),
            ],
        ));
    }
    if let Some(port) = spec.port {
        if !is_port_available(port) {
            append_app_log(
                &root,
                &format!(
                    "{} did not start: port {} was already in use.",
                    spec.name, port
                ),
            )?;
            return Ok(ActionResult::coded(
                false,
                "service.portOccupied",
                format!(
                    "Porta {} ocupada. Pare o processo externo ou altere a configuracao.",
                    port
                ),
                [("port", port.to_string())],
            ));
        }
    }
    if service == "apache" && cfg.https_enabled {
        if !is_port_available(cfg.https_port) {
            append_app_log(
                &root,
                &format!(
                    "Apache did not start: HTTPS port {} was already in use.",
                    cfg.https_port
                ),
            )?;
            return Ok(ActionResult::coded(
                false,
                "service.httpsPortOccupied",
                format!(
                    "Porta HTTPS {} ocupada. Outro processo está usando esta porta.",
                    cfg.https_port
                ),
                [("port", cfg.https_port.to_string())],
            ));
        }
        if cfg.https_port < 1024 {
            append_app_log(
                &root,
                &format!(
                    "Warning: HTTPS port {} requires administrator privileges on Windows.",
                    cfg.https_port
                ),
            )?;
        }
    }
    if service == "mysql" {
        ensure_mysql_data_initialized(&root, &spec)?;
        // Kill any stale mysqld.exe before starting (zombie from previous session locks ibdata1)
        #[cfg(target_os = "windows")]
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "mysqld.exe"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }

    if service == "apache" {
        // Kill any stale httpd.exe before starting (zombie child from previous session)
        #[cfg(target_os = "windows")]
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "httpd.exe"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        start_project_php_cgi(&root, &state)?;
        if let Some(message) = test_apache_config(&spec, &cfg.root_dir)? {
            append_app_log(&root, &message)?;
            return Ok(ActionResult::message(false, message));
        }
    }

    let mut command = Command::new(&spec.executable);
    command.current_dir(spec.work_dir);
    command.args(spec.args);
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command.spawn().map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(650));
    if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
        if service == "apache" {
            if let Some(port) = spec.port {
                if wait_until_port_busy(port, 8000) {
                    append_app_log(
                        &root,
                        "Apache started in external mode (parent process exited, port is active).",
                    )?;
                    return Ok(ActionResult::coded(
                        true,
                        "service.started",
                        "Apache iniciado.",
                        [("service", "Apache".to_string())],
                    ));
                }
            }
        }
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        let message = if stderr.trim().is_empty() {
            format!(
                "{} encerrou imediatamente com status {}.",
                spec.name, status
            )
        } else {
            format!("{} não iniciou: {}", spec.name, stderr.trim())
        };
        append_app_log(&root, &message)?;
        if service == "apache" {
            let mut children = state.children.lock().map_err(|e| e.to_string())?;
            stop_project_php_cgi(&mut children);
        }
        return Ok(ActionResult::message(false, message));
    }
    if service == "apache" {
        if let Some(port) = spec.port {
            if wait_until_port_busy(port, 8000) {
                append_app_log(&root, &format!("{} started", spec.name))?;
                return Ok(ActionResult::coded(
                    true,
                    "service.started",
                    format!("{} iniciado.", spec.name),
                    [("service", spec.name.to_string())],
                ));
            }
        }
        append_app_log(
            &root,
            "Apache did not confirm port listening after start (8s timeout).",
        )?;
        let mut children = state.children.lock().map_err(|e| e.to_string())?;
        stop_project_php_cgi(&mut children);
        return Ok(ActionResult::coded(
            false,
            "service.startTimeout",
            "Apache não confirmou escuta da porta após start.",
            [("service", "Apache".to_string())],
        ));
    }
    append_app_log(&root, &format!("{} started", spec.name))?;
    state
        .children
        .lock()
        .map_err(|e| e.to_string())?
        .insert(spec.id, child);

    Ok(ActionResult::coded(
        true,
        "service.started",
        format!("{} iniciado.", spec.name),
        [("service", spec.name.to_string())],
    ))
}

fn ensure_mysql_data_initialized(root: &Path, spec: &ServiceSpec) -> Result<(), String> {
    let data_dir = root.join("data").join("mysql");
    fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let marker = data_dir.join("ibdata1");
    if marker.exists() {
        return Ok(());
    }
    let has_partial = fs::read_dir(&data_dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false);
    if has_partial {
        append_app_log(
            root,
            "Partial MySQL data directory detected; recreating initial structure.",
        )?;
        for entry in fs::read_dir(&data_dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
            } else {
                fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
    }

    let conf = root.join("etc").join("mysql").join("my.ini");
    let conf_path = display_path(&conf);
    let data_path = display_path(&data_dir);
    let mut init = Command::new(&spec.executable);
    init.current_dir(&spec.work_dir)
        .args([
            &format!("--defaults-file={}", conf_path),
            &format!("--datadir={}", data_path),
            "--initialize-insecure",
            "--console",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    init.creation_flags(CREATE_NO_WINDOW);
    let output = init.output().map_err(|e| e.to_string())?;
    if output.status.success() && marker.exists() {
        append_app_log(root, "MySQL data directory initialized automatically.")?;
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "failure without details".to_string()
    };
    append_app_log(
        root,
        &format!("Failed to initialize MySQL data directory: {}", detail),
    )?;
    Err(format!("MySQL não pode inicializar datadir: {}", detail))
}

#[tauri::command]
fn stop_service(
    app: tauri::AppHandle,
    state: State<ProcessState>,
    service: String,
) -> Result<ActionResult, String> {
    stop_service_internal(app, &state, &service)
}

fn stop_service_internal(
    app: tauri::AppHandle,
    state: &State<ProcessState>,
    service: &str,
) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    let mut children = state.children.lock().map_err(|e| e.to_string())?;
    if service == "apache" {
        #[cfg(target_os = "windows")]
        {
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", "httpd.exe"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
        stop_project_php_cgi(&mut children);
        append_app_log(&root, "apache stopped")?;
        return Ok(ActionResult::coded(
            true,
            "service.stopped",
            "apache parado.",
            [("service", "apache".to_string())],
        ));
    }
    let child_opt = children.remove(service);
    let Some(mut child) = child_opt else {
        return Ok(ActionResult::coded(
            true,
            "service.alreadyStopped",
            "Servico ja estava parado.",
            [("service", service.to_string())],
        ));
    };

    let _ = child.kill();
    let _ = child.wait();
    if service == "apache" {
        stop_project_php_cgi(&mut children);
    }
    append_app_log(&root, &format!("{} stopped", service))?;
    Ok(ActionResult::coded(
        true,
        "service.stopped",
        format!("{} parado.", service),
        [("service", service.to_string())],
    ))
}

fn start_project_php_cgi(root: &Path, state: &State<ProcessState>) -> Result<(), String> {
    // Kill any stale php-cgi.exe from previous sessions so ports are available
    #[cfg(target_os = "windows")]
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "php-cgi.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let versions = list_projects_from_root(root)?
        .into_iter()
        .filter_map(|project| project.php_version)
        .collect::<std::collections::HashSet<_>>();

    for version in versions {
        let key = format!("php-cgi-{}", version);
        if state
            .children
            .lock()
            .map_err(|e| e.to_string())?
            .contains_key(&key)
        {
            continue;
        }
        let Some(php_dir) = php_runtime_dir_for_version(root, &version) else {
            append_app_log(
                root,
                &format!("PHP {} not found for php-cgi", version),
            )?;
            continue;
        };
        if let Err(err) = validate_php_runtime(&php_dir) {
            append_app_log(
                root,
                &format!("PHP {} is invalid for php-cgi: {}", version, err),
            )?;
            continue;
        }
        let php_cgi = php_dir.join("php-cgi.exe");
        if !php_cgi.exists() {
            append_app_log(
                root,
                &format!("php-cgi.exe missing at {}", display_path(&php_dir)),
            )?;
            continue;
        }
        let ini_version = php_runtime_version(&php_dir).unwrap_or_else(|| version.clone());
        ensure_php_ini_for_runtime(root, &ini_version, &php_dir)?;
        let port = php_cgi_port(&version);
        if !is_port_available(port) {
            append_app_log(
                root,
                &format!(
                    "php-cgi {} already uses or found occupied port {}",
                    version, port
                ),
            )?;
            continue;
        }
        let mut php_cmd = Command::new(&php_cgi);
        php_cmd
            .args([
                "-b",
                &format!("127.0.0.1:{}", port),
                "-c",
                &display_path(&php_ini_path(root, &ini_version)),
            ])
            .current_dir(&php_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        php_cmd.creation_flags(CREATE_NO_WINDOW);
        let child = php_cmd.spawn().map_err(|e| e.to_string())?;
        state
            .children
            .lock()
            .map_err(|e| e.to_string())?
            .insert(key, child);
        append_app_log(
            root,
            &format!("php-cgi {} started at 127.0.0.1:{}", version, port),
        )?;
    }

    Ok(())
}

fn test_apache_config(spec: &ServiceSpec, _root: &Path) -> Result<Option<String>, String> {
    let mut test = Command::new(&spec.executable);
    test.current_dir(&spec.work_dir);
    test.arg("-t").args(&spec.args);
    #[cfg(target_os = "windows")]
    test.creation_flags(CREATE_NO_WINDOW);
    let output = test.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(None);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "falha sem detalhes".to_string()
    };
    Ok(Some(format!("Apache não iniciou: {}", detail)))
}

fn native_path(path: &Path) -> String {
    #[cfg(target_os = "windows")]
    {
        return path.to_string_lossy().to_string();
    }
    #[allow(unreachable_code)]
    path.to_string_lossy().to_string()
}

fn stop_project_php_cgi(children: &mut HashMap<String, Child>) {
    let keys = children
        .keys()
        .filter(|key| key.starts_with("php-cgi-"))
        .cloned()
        .collect::<Vec<_>>();
    for key in keys {
        if let Some(mut child) = children.remove(&key) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[tauri::command]
fn restart_service(
    app: tauri::AppHandle,
    state: State<ProcessState>,
    service: String,
) -> Result<ActionResult, String> {
    restart_service_internal(app, &state, &service)
}

fn restart_service_internal(
    app: tauri::AppHandle,
    state: &State<ProcessState>,
    service: &str,
) -> Result<ActionResult, String> {
    let _ = stop_service_internal(app.clone(), state, service)?;
    start_service_internal(app, state, service)
}

fn service_is_running(
    state: &State<ProcessState>,
    service: &str,
    port: Option<u16>,
) -> Result<bool, String> {
    if service == "apache" {
        return Ok(port.map(|value| !is_port_available(value)).unwrap_or(false));
    }
    Ok(state
        .children
        .lock()
        .map_err(|e| e.to_string())?
        .contains_key(service))
}

#[tauri::command]
fn update_ports(app: tauri::AppHandle, ports: PortUpdate) -> Result<ActionResult, String> {
    validate_port(ports.http_port, "HTTP")?;
    validate_port(ports.https_port, "HTTPS")?;
    validate_port(ports.mysql_port, "MySQL")?;

    if ports.http_port == ports.https_port
        || ports.http_port == ports.mysql_port
        || ports.https_port == ports.mysql_port
    {
        return Err("As portas dos servicos precisam ser diferentes.".to_string());
    }

    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let mut cfg = read_config(&root)?;
    cfg.http_port = ports.http_port;
    cfg.https_port = ports.https_port;
    cfg.mysql_port = ports.mysql_port;
    write_config(&root, &cfg)?;
    ensure_mysql_config(&root)?;
    let _ = generate_apache_config(app);
    Ok(ActionResult::coded(
        true,
        "ports.updated",
        "Portas atualizadas. Reinicie os servicos para aplicar.",
        [
            ("http", ports.http_port.to_string()),
            ("https", ports.https_port.to_string()),
            ("mysql", ports.mysql_port.to_string()),
        ],
    ))
}

#[tauri::command]
fn update_https(app: tauri::AppHandle, request: HttpsUpdate) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let mut cfg = read_config(&root)?;
    cfg.https_enabled = request.enabled;
    write_config(&root, &cfg)?;

    if request.enabled {
        install_mkcert_ca(&root)?;
        for project in list_projects(app.clone())? {
            ensure_domain_certificate(&root, &project.domain)?;
        }
    }

    let _ = generate_apache_config(app);
    let message = if request.enabled {
            "HTTPS local habilitado. O mkcert instalou a CA local e os certificados foram preparados. Reinicie o Apache.".to_string()
        } else {
            "HTTPS local desabilitado. Reinicie o Apache para aplicar.".to_string()
        };
    Ok(ActionResult::coded(
        true,
        if request.enabled { "https.enabled" } else { "https.disabled" },
        message,
        [],
    ))
}

#[tauri::command]
fn read_logs(app: tauri::AppHandle, kind: String) -> Result<LogBundle, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let safe_kind = match kind.as_str() {
        "apache" | "mysql" | "php" | "app" => kind,
        _ => "app".to_string(),
    };
    let log_path = root
        .join("logs")
        .join(&safe_kind)
        .join(format!("{}.log", safe_kind));
    let content = if log_path.exists() {
        tail_file(&log_path, 300)?
    } else {
        format!("Nenhum log encontrado em {}", display_path(&log_path))
    };
    Ok(LogBundle {
        kind: safe_kind,
        content,
    })
}

#[tauri::command]
fn open_path(path: String) -> Result<ActionResult, String> {
    let target = PathBuf::from(path.replace('/', "\\"));
    if !target.exists() {
        return Err(format!("Pasta não encontrada: {}", display_path(&target)));
    }
    let target = target.canonicalize().unwrap_or(target);

    #[cfg(target_os = "windows")]
    let status = Command::new("explorer.exe")
        .arg(target.as_os_str())
        .status()
        .map_err(|e| e.to_string())?;
    #[cfg(not(target_os = "windows"))]
    let status = Command::new("xdg-open")
        .arg(target.as_os_str())
        .status()
        .map_err(|e| e.to_string())?;

    Ok(ActionResult::coded(
        status.success(),
        "path.opened",
        format!("Pasta aberta: {}.", display_path(&target)),
        [("path", display_path(&target))],
    ))
}

#[tauri::command]
fn open_www_folder(app: tauri::AppHandle) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    open_path(root.join("www").to_string_lossy().to_string())
}

#[tauri::command]
fn open_vhosts_folder(app: tauri::AppHandle) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    open_path(
        root.join("etc")
            .join("apache")
            .join("vhosts")
            .to_string_lossy()
            .to_string(),
    )
}

#[tauri::command]
fn open_url(url: String) -> Result<ActionResult, String> {
    #[cfg(target_os = "windows")]
    let status = Command::new("cmd")
        .args(["/C", "start", "", &url])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| e.to_string())?;
    #[cfg(not(target_os = "windows"))]
    let status = Command::new("xdg-open")
        .arg(url)
        .status()
        .map_err(|e| e.to_string())?;

    Ok(ActionResult::coded(
        status.success(),
        "url.opened",
        "URL solicitada ao navegador padrão.",
        [("url", url)],
    ))
}

#[tauri::command]
fn open_project(
    app: tauri::AppHandle,
    state: State<ProcessState>,
    domain: String,
) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let domain = normalize_domain(&domain)?;
    let cfg = read_config(&root)?;
    let project = list_projects(app.clone())?
        .into_iter()
        .find(|project| project.domain == domain)
        .ok_or_else(|| format!("Projeto {} não encontrado.", domain))?;
    let mut notes = Vec::new();

    if !project.host_configured {
        let host_result = add_host(app.clone(), domain.clone())?;
        notes.push(host_result.message);
        if !read_hosts_file().unwrap_or_default().contains(&domain) {
            return Ok(ActionResult::coded(
                false,
                "hosts.permissionRequired",
                format!(
                    "Confirme a permissão para adicionar {} ao hosts e tente abrir novamente.",
                    domain
                ),
                [("domain", domain)],
            ));
        }
    }

    let apache_spec =
        service_spec(&cfg, "apache").ok_or_else(|| "Servico Apache desconhecido.".to_string())?;
    if !apache_spec.executable.exists() {
        return Ok(ActionResult::coded(
            false,
            "apache.missingForProject",
            format!(
                "Apache não encontrado em {}. Instale o Apache pelo catálogo de pacotes antes de abrir {}.",
                display_path(&apache_spec.executable),
                domain
            ),
            [
                ("path", display_path(&apache_spec.executable)),
                ("domain", domain),
            ],
        ));
    }

    let apache_running = service_is_running(&state, "apache", apache_spec.port)?;
    if apache_running {
        generate_apache_config(app.clone())?;
        notes.push("Configurações regeneradas; Apache já estava em execução.".to_string());
    } else {
        let started = start_service(app.clone(), state.clone(), "apache".to_string())?;
        if !started.ok {
            return Ok(ActionResult::coded(
                false,
                "apache.startFailed",
                format!("Não foi possível iniciar o Apache: {}", started.message),
                [("message", started.message)],
            ));
        }
        notes.push(started.message);
    }

    let url = if cfg.https_enabled && project.ssl_enabled {
        project.https_url
    } else {
        project.http_url
    };
    let opened = open_url(url.clone())?;
    notes.push(format!("Abrindo {}.", url));

    Ok(ActionResult::message(opened.ok, notes.join(" ")))
}

#[tauri::command]
fn open_vhost_file(app: tauri::AppHandle, domain: String) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    generate_apache_config(app.clone())?;
    let domain = normalize_domain(&domain)?;
    let path = root
        .join("etc")
        .join("apache")
        .join("vhosts")
        .join(format!("{}.conf", domain));
    if !path.exists() {
        return Err(format!(
            "VirtualHost não encontrado: {}",
            display_path(&path)
        ));
    }
    open_text_file(&root, &path)?;
    Ok(ActionResult::coded(
        true,
        "vhost.opened",
        format!("VirtualHost aberto: {}.", display_path(&path)),
        [("path", display_path(&path))],
    ))
}

#[tauri::command]
fn list_packages(app: tauri::AppHandle) -> Result<Vec<PackageEntry>, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    seed_package_catalog(&app, &root)?;
    let path = package_catalog_path(&root);
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("Não foi possível ler {}: {}", display_path(&path), e))?;
    Ok(parse_package_catalog(&raw)
        .into_iter()
        .map(|mut entry| {
            let target = package_target_dir(&root, &entry.name, &entry.category, &entry.url);
            entry.install_dir = display_path(&target);
            entry.installed = package_is_installed(&target, &entry.name);
            entry
        })
        .collect())
}

#[tauri::command]
fn install_package(
    app: tauri::AppHandle,
    name: String,
    url: String,
    category: String,
) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    emit_install_progress(
        &app,
        &name,
        "package",
        "Preparando instalação",
        5,
        "running",
    );
    match install_package_internal(Some(&app), &root, &name, &url, &category) {
        Ok(result) => Ok(result),
        Err(err) => {
            emit_install_progress(&app, &name, "package", &err, 100, "error");
            Err(err)
        }
    }
}

fn install_package_internal(
    app: Option<&tauri::AppHandle>,
    root: &Path,
    name: &str,
    url: &str,
    category: &str,
) -> Result<ActionResult, String> {
    let safe_name = slugify(&name);
    if safe_name.is_empty() {
        return Err("Pacote invalido.".to_string());
    }

    let target_dir = package_target_dir(&root, &name, &category, &url);
    if package_is_installed(&target_dir, &name) {
        if let Some(app) = app {
            emit_install_progress(app, name, "package", "Pacote ja instalado", 100, "done");
        }
        return Ok(ActionResult::coded(
            true,
            "package.alreadyInstalled",
            format!(
                "{} já está instalado em {}.",
                name,
                display_path(&target_dir)
            ),
            [
                ("package", name.to_string()),
                ("path", display_path(&target_dir)),
            ],
        ));
    }
    if let Some(app) = app {
        emit_install_progress(
            app,
            name,
            "package",
            "Criando pasta de destino",
            10,
            "running",
        );
    }
    fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;

    let downloads = root.join("tmp").join("downloads");
    fs::create_dir_all(&downloads).map_err(|e| e.to_string())?;
    let extension = package_extension(&url);
    let archive = downloads.join(format!("{}{}", safe_name, extension));

    if let Some(app) = app {
        emit_install_progress(app, name, "package", "Baixando pacote", 15, "running");
        download_file_with_progress(&url, &archive, |percent| {
            emit_install_progress(app, name, "package", "Baixando pacote", percent, "running");
        })?;
    } else {
        download_file(&url, &archive)?;
    }
    if extension.eq_ignore_ascii_case(".zip") {
        if let Some(app) = app {
            emit_install_progress(app, name, "package", "Extraindo arquivos", 65, "running");
            expand_zip_with_progress(&archive, &target_dir, |percent| {
                emit_install_progress(
                    app,
                    name,
                    "package",
                    "Extraindo arquivos",
                    percent,
                    "running",
                );
            })?;
        } else {
            expand_zip(&archive, &target_dir)?;
        }
        flatten_single_directory(&target_dir)?;
    } else {
        if let Some(app) = app {
            emit_install_progress(app, name, "package", "Copiando arquivo", 78, "running");
        }
        fs::copy(
            &archive,
            target_dir.join(archive.file_name().unwrap_or_default()),
        )
        .map_err(|e| e.to_string())?;
    }

    if let Some(app) = app {
        emit_install_progress(
            app,
            name,
            "package",
            "Ajustando estrutura Laragon",
            88,
            "running",
        );
    }
    if name.to_ascii_lowercase().starts_with("apache") {
        flatten_named_child(&target_dir, "Apache24")?;
    }
    flatten_single_directory(&target_dir)?;

    append_app_log(&root, &format!("Package installed from catalog: {}", name))?;
    if let Some(app) = app {
        emit_install_progress(app, name, "package", "Instalação concluída", 100, "done");
    }
    Ok(ActionResult::coded(
        true,
        "package.installed",
        format!("{} instalado em {}.", name, display_path(&target_dir)),
        [
            ("package", name.to_string()),
            ("path", display_path(&target_dir)),
        ],
    ))
}

#[tauri::command]
fn list_site_templates(app: tauri::AppHandle) -> Result<Vec<SiteTemplate>, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    seed_site_template_catalog(&app, &root)?;
    let path = site_template_catalog_path(&root);
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("Não foi possível ler {}: {}", display_path(&path), e))?;
    Ok(parse_site_template_catalog(&raw))
}

#[tauri::command]
fn list_php_options(app: tauri::AppHandle) -> Result<Vec<PhpOption>, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    seed_package_catalog(&app, &root)?;
    let raw = fs::read_to_string(package_catalog_path(&root)).map_err(|e| e.to_string())?;
    let packages = parse_package_catalog(&raw);
    let mut options = packages
        .into_iter()
        .filter(|entry| entry.name.to_ascii_lowercase().starts_with("php-"))
        .filter_map(|entry| {
            let version = entry.name.trim_start_matches("PHP-").to_string();
            let target = package_target_dir(&root, &entry.name, &entry.category, &entry.url);
            let installed = package_is_installed(&target, &entry.name);
            Some(PhpOption {
                label: entry.name,
                version,
                installed,
                installable: !entry.url.trim().is_empty(),
            })
        })
        .collect::<Vec<_>>();
    options.sort_by(|a, b| compare_versions_desc(&a.version, &b.version));
    Ok(options)
}

#[tauri::command]
fn list_php_runtimes(app: tauri::AppHandle) -> Result<Vec<PhpRuntimeInfo>, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let mut runtimes = installed_php_runtimes(&root)?;
    runtimes.sort_by(|a, b| compare_versions_desc(&a.version, &b.version));
    Ok(runtimes)
}

#[tauri::command]
fn list_php_extensions(
    app: tauri::AppHandle,
    version: String,
) -> Result<Vec<PhpExtensionInfo>, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let php_dir = php_runtime_dir_for_version(&root, &version)
        .ok_or_else(|| format!("PHP {} não está instalado.", version))?;
    ensure_php_ini_for_runtime(&root, &version, &php_dir)?;
    let ini = fs::read_to_string(php_ini_path(&root, &version)).unwrap_or_default();
    let mut extensions = fs::read_dir(php_dir.join("ext"))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let file = path.file_name()?.to_string_lossy().to_string();
            if !file.to_ascii_lowercase().starts_with("php_")
                || !file.to_ascii_lowercase().ends_with(".dll")
            {
                return None;
            }
            let name = file
                .trim_start_matches("php_")
                .trim_end_matches(".dll")
                .to_string();
            Some(PhpExtensionInfo {
                enabled: php_extension_enabled(&ini, &file, &name),
                name,
                dll: file,
            })
        })
        .collect::<Vec<_>>();
    extensions.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(extensions)
}

#[tauri::command]
fn set_php_extension(
    app: tauri::AppHandle,
    version: String,
    extension: String,
    enabled: bool,
) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let php_dir = php_runtime_dir_for_version(&root, &version)
        .ok_or_else(|| format!("PHP {} não está instalado.", version))?;
    ensure_php_ini_for_runtime(&root, &version, &php_dir)?;
    let ini_path = php_ini_path(&root, &version);
    let raw = fs::read_to_string(&ini_path).map_err(|e| e.to_string())?;
    let dll = normalize_php_extension_dll(&extension);
    let name = dll.trim_start_matches("php_").trim_end_matches(".dll");
    let updated = set_php_extension_line(&raw, &dll, name, enabled);
    fs::write(&ini_path, updated).map_err(|e| e.to_string())?;
    append_app_log(
        &root,
        &format!(
            "PHP {} extension {} {}",
            version,
            name,
            if enabled {
                "enabled"
            } else {
                "disabled"
            }
        ),
    )?;
    Ok(ActionResult::coded(
        true,
        if enabled { "php.extensionEnabled" } else { "php.extensionDisabled" },
        format!(
            "Extensão {} {} no PHP {}. Reinicie o Apache para aplicar.",
            name,
            if enabled {
                "habilitada"
            } else {
                "desabilitada"
            },
            version
        ),
        [
            ("extension", name.to_string()),
            ("version", version),
        ],
    ))
}

#[tauri::command]
fn open_php_ini(app: tauri::AppHandle, version: String) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let php_dir = php_runtime_dir_for_version(&root, &version)
        .ok_or_else(|| format!("PHP {} não está instalado.", version))?;
    ensure_php_ini_for_runtime(&root, &version, &php_dir)?;
    let ini = php_ini_path(&root, &version);
    open_text_file(&root, &ini)?;
    Ok(ActionResult::coded(
        true,
        "php.iniOpened",
        format!("php.ini do PHP {} aberto no editor.", version),
        [("version", version)],
    ))
}

#[tauri::command]
fn open_packages_config(app: tauri::AppHandle) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    seed_package_catalog(&app, &root)?;
    let packages = package_catalog_path(&root);
    if !packages.exists() {
        return Err(format!(
            "Arquivo não encontrado: {}",
            display_path(&packages)
        ));
    }

    open_text_file(&root, &packages)?;
    Ok(ActionResult::coded(
        true,
        "catalog.packagesOpened",
        "Catálogo de pacotes aberto no editor.",
        [("path", display_path(&packages))],
    ))
}

#[tauri::command]
fn open_hosts_file(app: tauri::AppHandle) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    let hosts = windows_hosts_path();

    open_text_file(&root, &hosts)?;
    Ok(ActionResult::coded(
        true,
        "hosts.opened",
        "Hosts aberto no editor.",
        [("path", display_path(&hosts))],
    ))
}

#[tauri::command]
fn open_sites_config(app: tauri::AppHandle) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    seed_site_template_catalog(&app, &root)?;
    let sites = site_template_catalog_path(&root);

    open_text_file(&root, &sites)?;
    Ok(ActionResult::coded(
        true,
        "catalog.templatesOpened",
        "Catálogo de modelos aberto no editor.",
        [("path", display_path(&sites))],
    ))
}

fn open_text_file(root: &Path, path: &Path) -> Result<(), String> {
    if let Some(npp) = tool_executable(root, "notepadpp") {
        Command::new(npp)
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    } else {
        Command::new("notepad")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn list_local_tools(app: tauri::AppHandle) -> Result<Vec<ToolInfo>, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    Ok(bundled_tool_specs(&app, &root)
        .into_iter()
        .map(|spec| ToolInfo {
            id: spec.id.to_string(),
            name: spec.name.to_string(),
            kind: spec.kind.to_string(),
            source_path: display_path(&spec.source_path),
            install_path: display_path(&spec.install_path),
            installed: spec.executable_path.exists(),
            available_source: spec.source_path.exists() || spec.source_url.is_some(),
        })
        .collect())
}

#[tauri::command]
fn install_local_tool(app: tauri::AppHandle, tool: String) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let spec = bundled_tool_specs(&app, &root)
        .into_iter()
        .find(|spec| spec.id == tool)
        .ok_or_else(|| "Ferramenta desconhecida.".to_string())?;

    emit_install_progress(
        &app,
        spec.name,
        "tool",
        "Preparando ferramenta",
        8,
        "running",
    );
    if !spec.source_path.exists() {
        let Some(url) = spec.source_url.as_deref() else {
            let err = format!(
                "Dependência não encontrada: {}",
                display_path(&spec.source_path)
            );
            emit_install_progress(&app, spec.name, "tool", &err, 100, "error");
            return Err(err);
        };
        if let Some(parent) = spec.source_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        emit_install_progress(&app, spec.name, "tool", "Baixando ferramenta", 15, "running");
        download_file_with_progress(url, &spec.source_path, |percent| {
            emit_install_progress(&app, spec.name, "tool", "Baixando ferramenta", percent, "running");
        })?;
    }
    if spec.executable_path.exists() {
        emit_install_progress(
            &app,
            spec.name,
            "tool",
            "Atualizando instalação existente",
            14,
            "running",
        );
        if let Err(e) = fs::remove_dir_all(&spec.install_path) {
            let err = format!(
                "Não foi possível atualizar {}. Feche a ferramenta se ela estiver aberta: {}",
                spec.name, e
            );
            emit_install_progress(&app, spec.name, "tool", &err, 100, "error");
            return Err(err);
        }
    }
    emit_install_progress(
        &app,
        spec.name,
        "tool",
        "Criando pasta de destino",
        18,
        "running",
    );
    fs::create_dir_all(&spec.install_path).map_err(|e| e.to_string())?;

    let result = if spec.id == "heidisql" {
        emit_install_progress(
            &app,
            spec.name,
            "tool",
            "Extraindo portable isolado",
            35,
            "running",
        );
        install_heidisql_isolated_with_progress(&spec, |percent| {
            emit_install_progress(
                &app,
                spec.name,
                "tool",
                "Extraindo portable isolado",
                percent,
                "running",
            );
        })
    } else if spec
        .source_path
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        emit_install_progress(&app, spec.name, "tool", "Extraindo arquivos", 35, "running");
        expand_zip_with_progress(&spec.source_path, &spec.install_path, |percent| {
            emit_install_progress(
                &app,
                spec.name,
                "tool",
                "Extraindo arquivos",
                percent,
                "running",
            );
        })
        .and_then(|_| flatten_single_directory(&spec.install_path))
    } else {
        emit_install_progress(&app, spec.name, "tool", "Copiando arquivo", 65, "running");
        fs::copy(
            &spec.source_path,
            spec.install_path
                .join(spec.source_path.file_name().unwrap_or_default()),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    };

    if let Err(err) = result {
        emit_install_progress(&app, spec.name, "tool", &err, 100, "error");
        return Err(err);
    }

    append_app_log(&root, &format!("Tool installed: {}", spec.name))?;
    emit_install_progress(&app, spec.name, "tool", "Ferramenta pronta", 100, "done");
    Ok(ActionResult::coded(
        true,
        "tool.installed",
        format!(
            "{} instalado em {}.",
            spec.name,
            display_path(&spec.install_path)
        ),
        [
            ("tool", spec.name.to_string()),
            ("path", display_path(&spec.install_path)),
        ],
    ))
}

#[tauri::command]
fn launch_tool(app: tauri::AppHandle, tool: String) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let spec = bundled_tool_specs(&app, &root)
        .into_iter()
        .find(|spec| spec.id == tool)
        .ok_or_else(|| "Ferramenta desconhecida.".to_string())?;

    if !spec.executable_path.exists() {
        return Err(format!(
            "{} ainda não está instalado. Use instalar primeiro.",
            spec.name
        ));
    }
    Command::new(&spec.executable_path)
        .current_dir(&spec.install_path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(ActionResult::coded(
        true,
        "tool.opened",
        format!("{} aberto.", spec.name),
        [("tool", spec.name.to_string())],
    ))
}

#[tauri::command]
fn add_host(app: tauri::AppHandle, domain: String) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    let domain = normalize_domain(&domain)?;
    let line = format!("127.0.0.1 {}", domain);
    let hosts_path = windows_hosts_path();
    let current = read_hosts_file().unwrap_or_default();

    if current.lines().any(|line| line.contains(&domain)) {
        return Ok(ActionResult::coded(
            true,
            "hosts.alreadyExists",
            format!("{} ja existe no hosts.", domain),
            [("domain", domain)],
        ));
    }

    let mut file = fs::OpenOptions::new().append(true).open(&hosts_path);

    match file.as_mut() {
        Ok(file) => {
            writeln!(file, "\n# Ipeenv\n{}", line).map_err(|e| e.to_string())?;
            append_app_log(&root, &format!("Host added: {}", domain))?;
            Ok(ActionResult::coded(
                true,
                "hosts.added",
                format!("{} adicionado ao hosts.", domain),
                [("domain", domain)],
            ))
        }
        Err(err) if is_permission_error(err) => {
            run_elevated_hosts_update(&root, &domain, true)?;
            append_app_log(
                &root,
                &format!("Elevation requested to add host: {}", domain),
            )?;
            Ok(ActionResult::coded(
                true,
                "hosts.addElevationRequested",
                format!("Permissão elevada solicitada para adicionar {} ao hosts. Confirme o UAC e recarregue o navegador.", domain),
                [("domain", domain)],
            ))
        }
        Err(err) => Err(format!(
            "Não foi possível editar {}: {}",
            display_path(&hosts_path),
            err
        )),
    }
}

#[tauri::command]
fn remove_host(app: tauri::AppHandle, domain: String) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    let domain = normalize_domain(&domain)?;
    let hosts_path = windows_hosts_path();
    let current = read_hosts_file().map_err(|e| e.to_string())?;
    let filtered = current
        .lines()
        .filter(|line| !line.contains(&domain))
        .collect::<Vec<_>>()
        .join("\n");
    match fs::write(&hosts_path, filtered) {
        Ok(()) => {
            append_app_log(&root, &format!("Host removed: {}", domain))?;
            Ok(ActionResult::coded(
                true,
                "hosts.removed",
                format!("{} removido do hosts.", domain),
                [("domain", domain)],
            ))
        }
        Err(err) if is_permission_error(&err) => {
            run_elevated_hosts_update(&root, &domain, false)?;
            append_app_log(
                &root,
                &format!("Elevation requested to remove host: {}", domain),
            )?;
            Ok(ActionResult::coded(
                true,
                "hosts.removeElevationRequested",
                format!(
                    "Permissão elevada solicitada para remover {} do hosts. Confirme o UAC.",
                    domain
                ),
                [("domain", domain)],
            ))
        }
        Err(err) => Err(format!("Não foi possível editar hosts: {}", err)),
    }
}

#[tauri::command]
fn enable_ssl(app: tauri::AppHandle, domain: String) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let domain = normalize_domain(&domain)?;
    ensure_domain_certificate(&root, &domain)?;
    append_app_log(&root, &format!("SSL prepared for {}", domain))?;
    Ok(ActionResult::coded(
        true,
        "ssl.prepared",
        format!("SSL local preparado para {}.", domain),
        [("domain", domain)],
    ))
}

#[tauri::command]
fn generate_apache_config(app: tauri::AppHandle) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    ensure_environment(&root)?;
    let cfg = read_config(&root)?;
    let projects = list_projects(app.clone())?;
    let apache_dir = root.join("etc").join("apache");
    let vhosts_dir = apache_dir.join("vhosts");
    fs::create_dir_all(&vhosts_dir).map_err(|e| e.to_string())?;
    let apache_root = active_runtime_dir(&root.join("bin").join("apache"), "httpd.exe")
        .unwrap_or_else(|| root.join("bin").join("apache"));
    let mime_types = apache_root.join("conf").join("mime.types");
    let types_config = if mime_types.exists() {
        format!("TypesConfig \"{}\"\n", display_path(&mime_types))
    } else {
        String::new()
    };
    let module_lines = apache_load_modules(&apache_root);
    let https_listen = if cfg.https_enabled {
        format!("Listen {}\n", cfg.https_port)
    } else {
        String::new()
    };

    let httpd = format!(
        "ServerRoot \"{}\"\n{}\nListen {}\n{}ServerName localhost\nDocumentRoot \"{}\"\nDirectoryIndex index.php index.html\n{}\nErrorLog \"{}\"\nLogFormat \"%h %l %u %t \\\"%r\\\" %>s %b\" common\nCustomLog \"{}\" common\nIncludeOptional \"{}\"\n",
        display_path(&apache_root),
        module_lines,
        cfg.http_port,
        https_listen,
        display_path(&root.join("www")),
        types_config,
        display_path(&root.join("logs").join("apache").join("error.log")),
        display_path(&root.join("logs").join("apache").join("access.log")),
        display_path(&vhosts_dir.join("*.conf")),
    );
    fs::write(apache_dir.join("httpd.conf"), &httpd).map_err(|e| e.to_string())?;
    let runtime_conf = apache_runtime_config_path(&apache_root);
    if let Some(parent) = runtime_conf.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(runtime_conf, &httpd).map_err(|e| e.to_string())?;

    for project in projects {
        if cfg.https_enabled {
            ensure_domain_certificate(&root, &project.domain)?;
        }
        let docroot = framework_docroot(&project.path, &project.framework);
        let project_root_dir = display_path(&PathBuf::from(&project.path));
        let parent_override_guard = if docroot != project_root_dir {
            format!(
                "  <Directory \"{}\">\n    AllowOverride None\n    Options -Indexes +FollowSymLinks\n    Require all granted\n  </Directory>\n",
                project_root_dir
            )
        } else {
            String::new()
        };
        let php_handler = project
            .php_version
            .as_deref()
            .map(|version| {
                format!(
                    "  ProxyFCGIBackendType GENERIC\n  <FilesMatch \"\\\\.php$\">\n    SetHandler \"proxy:fcgi://127.0.0.1:{}/\"\n  </FilesMatch>\n  ProxyFCGISetEnvIf \"true\" DOCUMENT_ROOT \"{}\"\n  ProxyFCGISetEnvIf \"true\" SCRIPT_FILENAME \"%{{reqenv:DOCUMENT_ROOT}}%{{reqenv:SCRIPT_NAME}}\"\n  ProxyFCGISetEnvIf \"true\" PATH_TRANSLATED \"%{{reqenv:DOCUMENT_ROOT}}%{{reqenv:SCRIPT_NAME}}\"\n",
                    php_cgi_port(version),
                    docroot,
                )
            })
            .unwrap_or_default();
        let mut conf = format!(
            "<VirtualHost *:{}>\n  ServerName {}\n  DocumentRoot \"{}\"\n{}  <Directory \"{}\">\n    AllowOverride All\n    Options -Indexes +FollowSymLinks\n    Require all granted\n  </Directory>\n{}</VirtualHost>\n",
            cfg.http_port,
            project.domain,
            docroot,
            parent_override_guard,
            docroot,
            php_handler,
        );
        if cfg.https_enabled {
            conf.push_str(&format!(
                "\n<VirtualHost *:{}>\n  ServerName {}\n  DocumentRoot \"{}\"\n{}  <Directory \"{}\">\n    AllowOverride All\n    Options -Indexes +FollowSymLinks\n    Require all granted\n  </Directory>\n{}  SSLEngine on\n  SSLCertificateFile \"{}\"\n  SSLCertificateKeyFile \"{}\"\n</VirtualHost>\n",
                cfg.https_port,
                project.domain,
                docroot,
                parent_override_guard,
                docroot,
                php_handler,
                display_path(&root.join("etc").join("ssl").join("certs").join(format!("{}.crt", project.domain))),
                display_path(&root.join("etc").join("ssl").join("certs").join(format!("{}.key", project.domain))),
            ));
        }
        fs::write(vhosts_dir.join(format!("{}.conf", project.domain)), conf)
            .map_err(|e| e.to_string())?;
    }

    Ok(ActionResult::coded(
        true,
        "apache.configGenerated",
        "Configuracao do Apache regenerada.",
        [],
    ))
}

struct ServiceSpec {
    id: String,
    name: String,
    executable: PathBuf,
    work_dir: PathBuf,
    args: Vec<String>,
    port: Option<u16>,
    version: String,
}

fn service_catalog(cfg: &AppConfig, state: &ProcessState) -> Vec<ServiceInfo> {
    let running = state.children.lock().ok();
    ["apache", "mysql"]
        .iter()
        .filter_map(|id| service_spec(cfg, id))
        .map(|spec| {
            let pid = running
                .as_ref()
                .and_then(|map| map.get(&spec.id))
                .map(|child| child.id());
            let externally_running =
                spec.id == "apache" && spec.port.map(|p| !is_port_available(p)).unwrap_or(false);
            let status = if pid.is_some() || externally_running {
                "running"
            } else {
                "stopped"
            }
            .to_string();
            let is_enabled = cfg.enabled_services.contains(&spec.id);
            ServiceInfo {
                id: spec.id,
                name: spec.name,
                version: spec.version,
                port: spec.port,
                status,
                pid,
                available: spec.executable.exists(),
                port_available: spec.port.map(is_port_available),
                executable: display_path(&spec.executable),
                last_message: if spec.executable.exists() {
                    "Pronto para iniciar".to_string()
                } else {
                    "Binário ainda não empacotado".to_string()
                },
                enabled: is_enabled,
            }
        })
        .collect()
}

fn service_spec(cfg: &AppConfig, id: &str) -> Option<ServiceSpec> {
    let bin = cfg.root_dir.join("bin");
    match id {
        "apache" => {
            let apache_root = active_runtime_dir(&bin.join("apache"), "httpd.exe")
                .unwrap_or_else(|| bin.join("apache"));
            Some(ServiceSpec {
                id: "apache".to_string(),
                name: "Apache".to_string(),
                executable: apache_root.join("bin").join("httpd.exe"),
                work_dir: apache_root.clone(),
                args: vec![
                    "-d".to_string(),
                    native_path(&apache_root),
                    "-f".to_string(),
                    "conf/ipeenv-httpd.conf".to_string(),
                ],
                port: Some(cfg.http_port),
                version: "Apache 2.4.x".to_string(),
            })
        }
        "mysql" => Some(ServiceSpec {
            id: "mysql".to_string(),
            name: "MySQL".to_string(),
            executable: active_runtime_dir(&bin.join("mysql"), "mysqld.exe")
                .unwrap_or_else(|| bin.join("mysql"))
                .join("bin")
                .join("mysqld.exe"),
            work_dir: active_runtime_dir(&bin.join("mysql"), "mysqld.exe")
                .unwrap_or_else(|| bin.join("mysql")),
            args: vec![
                format!(
                    "--defaults-file={}",
                    display_path(&cfg.root_dir.join("etc").join("mysql").join("my.ini"))
                ),
                format!(
                    "--datadir={}",
                    display_path(&cfg.root_dir.join("data").join("mysql"))
                ),
            ],
            port: Some(cfg.mysql_port),
            version: "MySQL 8.x".to_string(),
        }),
        _ => None,
    }
}

fn apache_runtime_config_path(apache_root: &Path) -> PathBuf {
    apache_root.join("conf").join("ipeenv-httpd.conf")
}

fn app_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let workspace = workspace_root();
    if cfg!(debug_assertions) && workspace.join("mvp.md").exists() {
        return Ok(workspace);
    }

    let exe = env::current_exe().map_err(|e| e.to_string())?;
    if let Some(parent) = exe.parent() {
        return Ok(parent.to_path_buf());
    }

    app.path().app_data_dir().map_err(|e| e.to_string())
}

fn required_folders() -> Vec<&'static str> {
    vec![
        "bin/apache",
        "bin/php",
        "bin/mysql",
        "bin/openssl",
        "config",
        "dependencias",
        "data/mysql",
        "etc/apache/vhosts",
        "etc/php",
        "etc/mysql",
        "etc/ssl/certs",
        "etc/hosts",
        "logs/apache",
        "logs/php",
        "logs/mysql",
        "logs/app",
        "tmp",
        "www",
        "backup",
    ]
}

fn ensure_environment(root: &Path) -> Result<(), String> {
    for folder in required_folders() {
        fs::create_dir_all(root.join(folder)).map_err(|e| e.to_string())?;
    }
    let cfg_path = root.join("config").join("app.json");
    if !cfg_path.exists() {
        let cfg = AppConfig::default_for(root.to_path_buf());
        fs::write(
            &cfg_path,
            serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    let services_path = root.join("config").join("services.json");
    if !services_path.exists() {
        fs::write(
            &services_path,
            "{\n  \"apache\": { \"autostart\": false },\n  \"mysql\": { \"autostart\": false }\n}\n",
        )
        .map_err(|e| e.to_string())?;
    }
    ensure_mysql_config(root)?;
    ensure_php_config(root)?;
    Ok(())
}

fn migrate_flat_runtime_dirs(root: &Path) -> Result<(), String> {
    migrate_flat_runtime_dir(&root.join("bin").join("php"), "php.exe", RuntimeKind::Php)?;
    migrate_flat_runtime_dir(
        &root.join("bin").join("mysql"),
        "bin/mysqld.exe",
        RuntimeKind::Mysql,
    )?;
    migrate_flat_runtime_dir(
        &root.join("bin").join("apache"),
        "bin/httpd.exe",
        RuntimeKind::Apache,
    )?;
    rename_legacy_migrated_dir(&root.join("bin").join("php"), RuntimeKind::Php)?;
    rename_legacy_migrated_dir(&root.join("bin").join("mysql"), RuntimeKind::Mysql)?;
    rename_legacy_migrated_dir(&root.join("bin").join("apache"), RuntimeKind::Apache)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum RuntimeKind {
    Php,
    Mysql,
    Apache,
}

fn migrate_flat_runtime_dir(parent: &Path, marker: &str, kind: RuntimeKind) -> Result<(), String> {
    let marker_path = marker
        .split('/')
        .fold(parent.to_path_buf(), |path, part| path.join(part));
    if !marker_path.exists() {
        return Ok(());
    }

    let target = parent.join(runtime_folder_name(parent, kind));
    if target.exists() {
        return Ok(());
    }
    fs::create_dir_all(&target).map_err(|e| e.to_string())?;

    for entry in fs::read_dir(parent).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path == target {
            continue;
        }
        let dest = target.join(entry.file_name());
        fs::rename(&path, &dest).or_else(|_| {
            if path.is_dir() {
                copy_dir_contents(&path, &dest)
            } else {
                fs::copy(&path, &dest)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        })?;
    }
    Ok(())
}

fn rename_legacy_migrated_dir(parent: &Path, kind: RuntimeKind) -> Result<(), String> {
    let legacy = parent.join(match kind {
        RuntimeKind::Php => "php-migrated",
        RuntimeKind::Mysql => "mysql-migrated",
        RuntimeKind::Apache => "apache-migrated",
    });
    if !legacy.exists() {
        return Ok(());
    }

    let target = parent.join(runtime_folder_name(&legacy, kind));
    if target == legacy || target.exists() {
        return Ok(());
    }
    fs::rename(legacy, target).map_err(|e| e.to_string())
}

fn runtime_folder_name(dir: &Path, kind: RuntimeKind) -> String {
    match kind {
        RuntimeKind::Php => {
            php_runtime_folder_name(dir).unwrap_or_else(|| "php-runtime".to_string())
        }
        RuntimeKind::Mysql => {
            mysql_runtime_folder_name(dir).unwrap_or_else(|| "mysql-runtime".to_string())
        }
        RuntimeKind::Apache => {
            apache_runtime_folder_name(dir).unwrap_or_else(|| "apache-runtime".to_string())
        }
    }
}

fn php_runtime_folder_name(dir: &Path) -> Option<String> {
    let php = dir.join("php.exe");
    let output = Command::new(php).arg("-v").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.split_whitespace().nth(1)?;
    let nts = if text.to_ascii_uppercase().contains("NTS") {
        "-nts"
    } else {
        ""
    };
    Some(format!("php-{}{}-Win32-x64", version, nts))
}

fn mysql_runtime_folder_name(dir: &Path) -> Option<String> {
    let mysqld = dir.join("bin").join("mysqld.exe");
    let output = Command::new(mysqld).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    Some(format!("mysql-{}-winx64", version.trim_matches(',')))
}

fn apache_runtime_folder_name(dir: &Path) -> Option<String> {
    let httpd = dir.join("bin").join("httpd.exe");
    let output = Command::new(httpd).arg("-v").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text
        .split_whitespace()
        .find_map(|part| part.strip_prefix("Apache/"))?;
    Some(format!("httpd-{}-win64", version))
}

fn ensure_mysql_config(root: &Path) -> Result<(), String> {
    let port = read_config(root).map(|cfg| cfg.mysql_port).unwrap_or(3306);
    let path = root.join("etc").join("mysql").join("my.ini");
    fs::write(
        path,
        format!(
            "[mysqld]\nport={}\ndatadir={}\nlog-error={}\n",
            port,
            display_path(&root.join("data").join("mysql")),
            display_path(&root.join("logs").join("mysql").join("mysql.log")),
        ),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn ensure_php_config(root: &Path) -> Result<(), String> {
    let path = root.join("etc").join("php").join("php.ini");
    if !path.exists() {
        fs::write(
            path,
            format!(
                "display_errors=On\nerror_log={}\ndate.timezone=America/Sao_Paulo\n",
                display_path(&root.join("logs").join("php").join("php.log")),
            ),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn installed_php_runtimes(root: &Path) -> Result<Vec<PhpRuntimeInfo>, String> {
    let parent = root.join("bin").join("php");
    if !parent.exists() {
        return Ok(Vec::new());
    }

    fs::read_dir(parent)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("php.exe").exists())
        .filter(|path| validate_php_runtime(path).is_ok())
        .map(|path| {
            let version = php_runtime_version(&path)
                .or_else(|| {
                    path.file_name()
                        .map(|name| php_version_from_name(&name.to_string_lossy()))
                })
                .unwrap_or_else(|| "desconhecida".to_string());
            ensure_php_ini_for_runtime(root, &version, &path)?;
            let extension_count = fs::read_dir(path.join("ext"))
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter(|entry| {
                            entry.path().file_name().is_some_and(|name| {
                                name.to_string_lossy()
                                    .to_ascii_lowercase()
                                    .starts_with("php_")
                            })
                        })
                        .count()
                })
                .unwrap_or(0);
            Ok(PhpRuntimeInfo {
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| version.clone()),
                path: display_path(&path),
                ini_path: display_path(&php_ini_path(root, &version)),
                extension_count,
                version,
            })
        })
        .collect()
}

fn php_ini_path(root: &Path, version: &str) -> PathBuf {
    root.join("etc")
        .join("php")
        .join(format!("php-{}.ini", slugify(version)))
}

fn ensure_php_ini_for_runtime(root: &Path, version: &str, php_dir: &Path) -> Result<(), String> {
    let path = php_ini_path(root, version);
    let mut content = if path.exists() {
        fs::read_to_string(&path).unwrap_or_default()
    } else {
        let source = php_dir
            .join("php.ini-development")
            .exists()
            .then(|| php_dir.join("php.ini-development"))
            .or_else(|| {
                php_dir
                    .join("php.ini-production")
                    .exists()
                    .then(|| php_dir.join("php.ini-production"))
            })
            .or_else(|| {
                root.join("etc")
                    .join("php")
                    .join("php.ini")
                    .exists()
                    .then(|| root.join("etc").join("php").join("php.ini"))
            });

        source
            .and_then(|source| fs::read_to_string(source).ok())
            .unwrap_or_default()
    };

    content = set_ini_directive(&content, "date.timezone", "America/Sao_Paulo");
    content = set_ini_directive(
        &content,
        "extension_dir",
        &format!("\"{}\"", display_path(&php_dir.join("ext"))),
    );
    content = set_ini_directive(
        &content,
        "error_log",
        &display_path(
            &root
                .join("logs")
                .join("php")
                .join(format!("php-{}.log", slugify(version))),
        ),
    );
    content = set_ini_directive(&content, "allow_url_fopen", "On");

    for extension in default_php_extensions() {
        if php_dir
            .join("ext")
            .join(format!("php_{}.dll", extension))
            .exists()
        {
            content = set_php_extension_line(
                &content,
                &format!("php_{}.dll", extension),
                extension,
                true,
            );
        }
    }
    fs::write(path, content).map_err(|e| e.to_string())
}

fn default_php_extensions() -> &'static [&'static str] {
    &[
        "openssl",
        "curl",
        "fileinfo",
        "intl",
        "mbstring",
        "mysqli",
        "pdo_mysql",
        "zip",
        "gd",
    ]
}

fn set_ini_directive(raw: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut lines = raw
        .lines()
        .map(|line| {
            let uncommented = line.trim_start().trim_start_matches(';').trim_start();
            if uncommented.starts_with(key)
                && uncommented[key.len()..].trim_start().starts_with('=')
            {
                found = true;
                format!("{}={}", key, value)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>();
    if !found {
        lines.push(format!("{}={}", key, value));
    }
    lines.join("\n") + "\n"
}

fn php_runtime_version(path: &Path) -> Option<String> {
    let mut command = Command::new(path.join("php.exe"));
    command.arg("-r").arg("echo PHP_VERSION;");
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.output().ok()?;
    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !version.is_empty() {
            return Some(version);
        }
    }
    None
}

fn php_version_from_name(name: &str) -> String {
    name.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|part| part.chars().any(|c| c == '.'))
        .unwrap_or(name)
        .to_string()
}

fn normalize_php_extension_dll(extension: &str) -> String {
    let mut value = extension.trim().trim_start_matches(';').trim().to_string();
    if let Some(stripped) = value.strip_prefix("extension=") {
        value = stripped.trim().trim_matches('"').to_string();
    }
    if !value.to_ascii_lowercase().starts_with("php_") {
        value = format!("php_{}", value);
    }
    if !value.to_ascii_lowercase().ends_with(".dll") {
        value.push_str(".dll");
    }
    value
}

fn php_extension_enabled(ini: &str, dll: &str, name: &str) -> bool {
    ini.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.starts_with(';') && php_extension_line_matches(trimmed, dll, name)
    })
}

fn set_php_extension_line(raw: &str, dll: &str, name: &str, enabled: bool) -> String {
    let mut found = false;
    let mut lines = raw
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            let uncommented = trimmed.trim_start_matches(';').trim();
            if php_extension_line_matches(uncommented, dll, name) {
                found = true;
                if enabled {
                    format!("extension={}", name)
                } else {
                    format!(";extension={}", name)
                }
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>();

    if !found {
        lines.push(if enabled {
            format!("extension={}", name)
        } else {
            format!(";extension={}", name)
        });
    }
    lines.join("\n") + "\n"
}

fn php_extension_line_matches(line: &str, dll: &str, name: &str) -> bool {
    let Some(value) = line.strip_prefix("extension=") else {
        return false;
    };
    let value = value.trim().trim_matches('"').replace('\\', "/");
    let file = value.rsplit('/').next().unwrap_or(&value);
    file.eq_ignore_ascii_case(dll) || file.eq_ignore_ascii_case(name)
}

fn read_config(root: &Path) -> Result<AppConfig, String> {
    let path = root.join("config").join("app.json");
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

fn write_config(root: &Path, cfg: &AppConfig) -> Result<(), String> {
    let path = root.join("config").join("app.json");
    fs::write(
        path,
        serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn append_app_log(root: &Path, message: &str) -> Result<(), String> {
    let log_dir = root.join("logs").join("app");
    fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("app.log"))
        .map_err(|e| e.to_string())?;
    writeln!(
        file,
        "[{}] {}",
        Local::now().format("%Y-%m-%d %H:%M:%S"),
        message
    )
    .map_err(|e| e.to_string())
}

fn tail_file(path: &Path, max_lines: usize) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|e| e.to_string())?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    Ok(lines[start..].join("\n"))
}

fn windows_hosts_path() -> PathBuf {
    PathBuf::from(r"C:\Windows\System32\drivers\etc\hosts")
}

fn read_hosts_file() -> Result<String, std::io::Error> {
    fs::read_to_string(windows_hosts_path())
}

fn ensure_domain_certificate(root: &Path, domain: &str) -> Result<(), String> {
    let cert_dir = root.join("etc").join("ssl").join("certs");
    fs::create_dir_all(&cert_dir).map_err(|e| e.to_string())?;
    let cert = cert_dir.join(format!("{}.crt", domain));
    let key = cert_dir.join(format!("{}.key", domain));

    if cert.exists() && key.exists() && !certificate_is_placeholder(&cert) {
        return Ok(());
    }

    install_mkcert_ca(root)?;
    let mkcert = mkcert_path(root)?;
    let output = Command::new(&mkcert)
        .args([
            "-cert-file",
            &cert.to_string_lossy(),
            "-key-file",
            &key.to_string_lossy(),
            domain,
        ])
        .env("CAROOT", mkcert_ca_root(root))
        .env("TRUST_STORES", "system")
        .current_dir(&cert_dir)
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!(
            "mkcert não conseguiu gerar certificado para {}: {} {}",
            domain,
            stderr.trim(),
            stdout.trim()
        ))
    }
}

fn certificate_is_placeholder(cert: &Path) -> bool {
    fs::read_to_string(cert)
        .map(|content| content.contains("IPEENV LOCAL CERT PLACEHOLDER"))
        .unwrap_or(false)
}

fn install_mkcert_ca(root: &Path) -> Result<(), String> {
    let mkcert = mkcert_path(root)?;
    fs::create_dir_all(mkcert_ca_root(root)).map_err(|e| e.to_string())?;
    let output = Command::new(&mkcert)
        .arg("-install")
        .env("CAROOT", mkcert_ca_root(root))
        .env("TRUST_STORES", "system")
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{} {}", stdout.trim(), stderr.trim());
        if combined.contains("already installed in the system trust store") {
            Ok(())
        } else {
            Err(format!("mkcert -install falhou: {}", combined.trim()))
        }
    }
}

fn mkcert_ca_root(root: &Path) -> PathBuf {
    root.join("etc").join("ssl").join("mkcert")
}

fn mkcert_path(root: &Path) -> Result<PathBuf, String> {
    let candidates = [
        root.join("bin").join("mkcert").join("mkcert.exe"),
        root.join("dependencias").join("mkcert.exe"),
        workspace_root().join("dependencias").join("mkcert.exe"),
    ];
    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| {
            "mkcert.exe não encontrado. Coloque-o em dependencias/mkcert.exe.".to_string()
        })
}

fn apache_load_modules(apache_root: &Path) -> String {
    let modules = [
        ("access_compat_module", "mod_access_compat.so"),
        ("actions_module", "mod_actions.so"),
        ("alias_module", "mod_alias.so"),
        ("allowmethods_module", "mod_allowmethods.so"),
        ("authz_core_module", "mod_authz_core.so"),
        ("authz_host_module", "mod_authz_host.so"),
        ("dir_module", "mod_dir.so"),
        ("env_module", "mod_env.so"),
        ("headers_module", "mod_headers.so"),
        ("log_config_module", "mod_log_config.so"),
        ("mime_module", "mod_mime.so"),
        ("rewrite_module", "mod_rewrite.so"),
        ("setenvif_module", "mod_setenvif.so"),
        ("socache_shmcb_module", "mod_socache_shmcb.so"),
        ("ssl_module", "mod_ssl.so"),
        ("proxy_module", "mod_proxy.so"),
        ("proxy_fcgi_module", "mod_proxy_fcgi.so"),
    ];

    modules
        .into_iter()
        .filter_map(|(module, file)| {
            let path = apache_root.join("modules").join(file);
            path.exists()
                .then(|| format!("LoadModule {} \"{}\"", module, display_path(&path)))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_permission_error(error: &std::io::Error) -> bool {
    matches!(error.kind(), std::io::ErrorKind::PermissionDenied)
}

fn run_elevated_hosts_update(root: &Path, domain: &str, add: bool) -> Result<(), String> {
    let script_dir = root.join("tmp").join("elevated");
    fs::create_dir_all(&script_dir).map_err(|e| e.to_string())?;
    let script = script_dir.join(format!(
        "{}-host-{}.ps1",
        if add { "add" } else { "remove" },
        slugify(domain)
    ));
    let escaped_domain = domain.replace('\'', "''");
    let script_content = if add {
        format!(
            "$hosts = Join-Path $env:SystemRoot 'System32\\drivers\\etc\\hosts'\n$domain = '{}'\n$content = Get-Content -LiteralPath $hosts -Raw -ErrorAction SilentlyContinue\nif ($content -notmatch [regex]::Escape($domain)) {{ Add-Content -LiteralPath $hosts -Value \"`r`n# Ipeenv`r`n127.0.0.1 $domain\" }}\nipconfig /flushdns | Out-Null\n",
            escaped_domain
        )
    } else {
        format!(
            "$hosts = Join-Path $env:SystemRoot 'System32\\drivers\\etc\\hosts'\n$domain = '{}'\n$lines = Get-Content -LiteralPath $hosts -ErrorAction Stop | Where-Object {{ $_ -notmatch [regex]::Escape($domain) }}\nSet-Content -LiteralPath $hosts -Value $lines -Encoding ASCII\nipconfig /flushdns | Out-Null\n",
            escaped_domain
        )
    };
    fs::write(&script, script_content).map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    {
        let script_path = script.to_string_lossy().replace('"', "\\\"");
        let command = format!(
            "Start-Process -FilePath powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\"'",
            script_path
        );
        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &command,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("Não foi possível solicitar elevação para editar hosts.".to_string());
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = script;
        return Err("Edição elevada do hosts ainda só está implementada no Windows.".to_string());
    }

    Ok(())
}

fn is_port_available(port: u16) -> bool {
    if !TcpListener::bind(("0.0.0.0", port)).is_ok() {
        return false;
    }
    // On Windows, bind checks can produce false positives in some edge cases.
    // Confirm by attempting loopback connections on both IPv4 and IPv6.
    !port_accepts_connections(port)
}

fn wait_until_port_busy(port: u16, timeout_ms: u64) -> bool {
    let start = Instant::now();
    loop {
        if !is_port_available(port) {
            return true;
        }
        if start.elapsed() >= Duration::from_millis(timeout_ms) {
            return false;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn port_accepts_connections(port: u16) -> bool {
    for addr in [format!("127.0.0.1:{}", port), format!("[::1]:{}", port)] {
        if let Ok(addrs) = addr.to_socket_addrs() {
            for socket_addr in addrs {
                if TcpStream::connect_timeout(&socket_addr, Duration::from_millis(200)).is_ok() {
                    return true;
                }
            }
        }
    }
    false
}

fn slugify(input: &str) -> String {
    input
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn normalize_domain(domain: &str) -> Result<String, String> {
    let slug = domain.trim().trim_end_matches(".test");
    let slug = slugify(slug);
    if slug.is_empty() {
        return Err("Dominio invalido.".to_string());
    }
    Ok(format!("{}.test", slug))
}

fn version_at_least(version: &str, minimum: &str) -> bool {
    let parse = |value: &str| {
        value
            .split('.')
            .map(|part| part.parse::<u32>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let current = parse(version);
    let required = parse(minimum);
    let len = current.len().max(required.len());
    for index in 0..len {
        let left = *current.get(index).unwrap_or(&0);
        let right = *required.get(index).unwrap_or(&0);
        if left > right {
            return true;
        }
        if left < right {
            return false;
        }
    }
    true
}

fn version_at_most(version: &str, maximum: &str) -> bool {
    version_at_least(maximum, version)
}

fn load_template_by_name(root: &Path, selected: &str) -> Option<SiteTemplate> {
    let raw = fs::read_to_string(site_template_catalog_path(root)).ok()?;
    parse_site_template_catalog(&raw)
        .into_iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(selected))
}

fn resolve_compatible_php_version_range(
    root: &Path,
    minimum: Option<&str>,
    maximum: Option<&str>,
) -> Option<String> {
    let mut installed = installed_php_runtimes(root).ok()?;
    installed.sort_by(|a, b| compare_versions_desc(&a.version, &b.version));
    if let Some(found) = installed
        .iter()
        .find(|runtime| {
            minimum
                .map(|min| version_at_least(&runtime.version, min))
                .unwrap_or(true)
                && maximum
                    .map(|max| version_at_most(&runtime.version, max))
                    .unwrap_or(true)
        })
        .map(|runtime| runtime.version.clone())
    {
        return Some(found);
    }

    let raw = fs::read_to_string(package_catalog_path(root)).ok()?;
    let mut candidates = parse_package_catalog(&raw)
        .into_iter()
        .filter(|entry| entry.name.to_ascii_lowercase().starts_with("php-"))
        .filter_map(|entry| {
            let version = entry.name.trim_start_matches("PHP-").trim().to_string();
            let min_ok = minimum
                .map(|min| version_at_least(&version, min))
                .unwrap_or(true);
            let max_ok = maximum
                .map(|max| version_at_most(&version, max))
                .unwrap_or(true);
            if min_ok && max_ok && !entry.url.trim().is_empty() {
                Some(version)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| compare_versions_desc(a, b));
    candidates.into_iter().next()
}

fn detect_framework(path: &Path) -> String {
    if path.join("artisan").exists() {
        "Laravel".to_string()
    } else if path.join("wp-config.php").exists() || path.join("wp-content").exists() {
        "WordPress".to_string()
    } else if path.join("webroot").join("index.php").exists()
        || (path.join("config").join("app.php").exists() && path.join("webroot").exists())
    {
        "CakePHP".to_string()
    } else if path.join("composer.json").exists() {
        "PHP".to_string()
    } else {
        "Projeto".to_string()
    }
}

fn framework_docroot(project_path: &str, framework: &str) -> String {
    match framework {
        "Laravel" => format!("{}/public", project_path),
        "CakePHP" => format!("{}/webroot", project_path),
        _ => project_path.to_string(),
    }
}

fn create_project_from_template(
    app: &tauri::AppHandle,
    root: &Path,
    project_dir: &Path,
    name: &str,
    template_name: Option<&str>,
    php_version: Option<&str>,
) -> Result<(), String> {
    let template = template_name
        .filter(|value| !value.trim().is_empty())
        .and_then(|selected| {
            let raw = fs::read_to_string(site_template_catalog_path(root)).ok()?;
            parse_site_template_catalog(&raw)
                .into_iter()
                .find(|entry| entry.name.eq_ignore_ascii_case(selected))
        });

    match template {
        Some(entry)
            if entry.source.starts_with("http://") || entry.source.starts_with("https://") =>
        {
            fs::create_dir_all(project_dir).map_err(|e| e.to_string())?;
            let downloads = root.join("tmp").join("downloads");
            fs::create_dir_all(&downloads).map_err(|e| e.to_string())?;
            let archive = downloads.join(format!(
                "site-{}{}",
                slugify(&entry.name),
                package_extension(&entry.source)
            ));
            emit_project_progress(app, name, "Baixando template", 42, "running");
            download_file(&entry.source, &archive)?;
            if package_extension(&entry.source).eq_ignore_ascii_case(".zip") {
                emit_project_progress(app, name, "Extraindo template", 55, "running");
                expand_zip(&archive, project_dir)?;
                flatten_single_directory(project_dir)?;
            } else {
                fs::copy(
                    &archive,
                    project_dir.join(archive.file_name().unwrap_or_default()),
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        Some(entry) if entry.source.starts_with("inline:php") || entry.source.is_empty() => {
            create_blank_project(project_dir, name)
        }
        Some(entry) => {
            emit_project_progress(app, name, "Preparando Composer", 45, "running");
            run_site_command(app, root, project_dir, name, &entry.source, php_version)
        }
        None => create_blank_project(project_dir, name),
    }
}

fn emit_project_progress(
    app: &tauri::AppHandle,
    project: &str,
    step: &str,
    percent: u8,
    status: &str,
) {
    let _ = app.emit(
        "project-progress",
        ProjectProgress {
            project: project.to_string(),
            step: step.to_string(),
            percent,
            status: status.to_string(),
        },
    );
}

fn emit_install_progress(
    app: &tauri::AppHandle,
    item: &str,
    kind: &str,
    step: &str,
    percent: u8,
    status: &str,
) {
    let _ = app.emit(
        "install-progress",
        InstallProgress {
            item: item.to_string(),
            kind: kind.to_string(),
            step: step.to_string(),
            percent,
            status: status.to_string(),
        },
    );
}

fn create_blank_project(project_dir: &Path, name: &str) -> Result<(), String> {
    fs::create_dir_all(project_dir).map_err(|e| e.to_string())?;
    fs::write(
        project_dir.join("index.php"),
        format!(
            "<?php\nhttp_response_code(200);\necho '<h1>{}</h1><p>Ipeenv está servindo este projeto.</p>';\n",
            name
        ),
    )
    .map_err(|e| e.to_string())
}

fn run_site_command(
    app: &tauri::AppHandle,
    root: &Path,
    project_dir: &Path,
    name: &str,
    command: &str,
    php_version: Option<&str>,
) -> Result<(), String> {
    let fallback_parent = root.join("www");
    let parent = project_dir.parent().unwrap_or(fallback_parent.as_path());
    let prepared = command
        .replace("{name}", name)
        .replace("{project}", name)
        .replace("{path}", &display_path(project_dir));

    let mut command_process = if prepared
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("composer ")
    {
        composer_command(app, root, name, php_version, &prepared)?
    } else {
        let mut process = Command::new("cmd");
        process
            .args(["/C", &prepared])
            .env("PATH", composer_path(root, php_version)?);
        #[cfg(target_os = "windows")]
        process.creation_flags(CREATE_NO_WINDOW);
        process
    };
    command_process
        .current_dir(parent)
        .env("COMPOSER_HOME", root.join("usr").join("composer"));
    apply_php_runtime_env(root, php_version, &mut command_process)?;

    emit_project_progress(app, name, "Executando Composer", 58, "running");
    let mut output = command_process.output().map_err(|e| e.to_string())?;
    if !output.stdout.is_empty() {
        append_app_log(
            root,
            &format!(
                "Composer stdout: {}",
                String::from_utf8_lossy(&output.stdout)
            ),
        )?;
    }
    if !output.stderr.is_empty() {
        append_app_log(
            root,
            &format!(
                "Composer stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )?;
    }

    if !output.status.success()
        && is_composer_security_block_error(&output.stderr)
        && prepared
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("composer ")
    {
        append_app_log(root, "Composer blocked dependencies because of security advisories. Retrying with --no-security-blocking for a legacy template.")?;
        emit_project_progress(
            app,
            name,
            "Ajustando politica de seguranca do Composer",
            63,
            "running",
        );
        let retry_command = format!("{} --no-security-blocking --no-audit", prepared.trim());
        let mut retry = composer_command(app, root, name, php_version, &retry_command)?;
        retry
            .current_dir(parent)
            .env("COMPOSER_HOME", root.join("usr").join("composer"));
        apply_php_runtime_env(root, php_version, &mut retry)?;
        output = retry.output().map_err(|e| e.to_string())?;
        if !output.stdout.is_empty() {
            append_app_log(
                root,
                &format!(
                    "Composer stdout (retry): {}",
                    String::from_utf8_lossy(&output.stdout)
                ),
            )?;
        }
        if !output.stderr.is_empty() {
            append_app_log(
                root,
                &format!(
                    "Composer stderr (retry): {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
            )?;
        }
    }

    if output.status.success() && project_dir.exists() {
        emit_project_progress(app, name, "Projeto baixado pelo Composer", 68, "running");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("sem detalhes no stderr");
        Err(format!(
            "Comando do catálogo de modelos falhou: {} ({})",
            prepared, detail
        ))
    }
}

fn is_composer_security_block_error(stderr: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    text.contains("security advisories")
        || text.contains("no-security-blocking")
        || text.contains("block-insecure")
}

fn composer_command(
    app: &tauri::AppHandle,
    root: &Path,
    project_name: &str,
    php_version: Option<&str>,
    prepared: &str,
) -> Result<Command, String> {
    let php_dir = resolve_php_runtime_for_command(root, php_version)?;
    let php = php_dir.join("php.exe");
    if !php.exists() {
        return Err(format!(
            "php.exe não encontrado em {}.",
            display_path(&php_dir)
        ));
    }
    let ini_version = php_runtime_version(&php_dir)
        .unwrap_or_else(|| php_version.unwrap_or_default().to_string());
    ensure_php_ini_for_runtime(root, &ini_version, &php_dir)?;
    let composer = ensure_composer_phar(app, root, Some(project_name))?;
    let mut args = prepared.split_whitespace().collect::<Vec<_>>();
    if args
        .first()
        .is_some_and(|part| part.eq_ignore_ascii_case("composer"))
    {
        args.remove(0);
    }
    let mut command = Command::new(php);
    command
        .arg("-c")
        .arg(php_ini_path(root, &ini_version))
        .arg(composer)
        .args(args);
    Ok(command)
}

fn ensure_composer_phar(
    app: &tauri::AppHandle,
    root: &Path,
    project_name: Option<&str>,
) -> Result<PathBuf, String> {
    let composer_dir = root.join("usr").join("composer");
    fs::create_dir_all(&composer_dir).map_err(|e| e.to_string())?;
    let composer = composer_dir.join("composer.phar");
    if composer.exists() {
        return Ok(composer);
    }
    if let Some(project_name) = project_name {
        emit_project_progress(app, project_name, "Baixando composer.phar", 46, "running");
    }
    emit_install_progress(
        app,
        "Composer",
        "composer",
        "Baixando composer.phar",
        20,
        "running",
    );
    download_file_with_progress(
        "https://getcomposer.org/download/latest-stable/composer.phar",
        &composer,
        |percent| {
            emit_install_progress(
                app,
                "Composer",
                "composer",
                "Baixando composer.phar",
                percent,
                "running",
            );
            if let Some(project_name) = project_name {
                let project_percent =
                    46 + ((percent.saturating_sub(20) as u16 * 8 / 40).min(8) as u8);
                emit_project_progress(
                    app,
                    project_name,
                    "Baixando composer.phar",
                    project_percent,
                    "running",
                );
            }
        },
    )?;
    emit_install_progress(app, "Composer", "composer", "Composer pronto", 100, "done");
    if let Some(project_name) = project_name {
        emit_project_progress(app, project_name, "Composer pronto", 54, "running");
    }
    Ok(composer)
}

fn resolve_php_runtime_for_command(
    root: &Path,
    php_version: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(version) = php_version {
        return php_runtime_dir_for_version(root, version).ok_or_else(|| {
            format!(
                "PHP {} foi solicitado, mas não está instalado/válido para execução.",
                version
            )
        });
    }
    active_php_runtime_dir(root)
        .ok_or_else(|| "Nenhuma versão de PHP instalada para executar Composer.".to_string())
}

fn apply_php_runtime_env(
    root: &Path,
    php_version: Option<&str>,
    command_process: &mut Command,
) -> Result<(), String> {
    let php_dir = resolve_php_runtime_for_command(root, php_version)?;
    let ini_version = php_runtime_version(&php_dir)
        .unwrap_or_else(|| php_version.unwrap_or_default().to_string());
    ensure_php_ini_for_runtime(root, &ini_version, &php_dir)?;
    command_process
        .env("PHPRC", php_ini_path(root, &ini_version))
        .env("PATH", composer_path(root, php_version)?);
    Ok(())
}

fn composer_path(root: &Path, php_version: Option<&str>) -> Result<String, String> {
    let mut paths = Vec::new();
    let php_dir = resolve_php_runtime_for_command(root, php_version)?;
    paths.push(display_path(&php_dir));
    paths.push(display_path(&root.join("bin").join("composer")));
    paths.push(env::var("PATH").unwrap_or_default());
    Ok(paths.join(";"))
}

fn ensure_php_version_installed(
    app: &tauri::AppHandle,
    root: &Path,
    version: &str,
) -> Result<(), String> {
    if php_version_installed(root, version) {
        return Ok(());
    }
    let raw = fs::read_to_string(package_catalog_path(root)).map_err(|e| e.to_string())?;
    let package = parse_package_catalog(&raw)
        .into_iter()
        .find(|entry| {
            entry.name.to_ascii_lowercase().starts_with("php-")
                && entry.name.trim_start_matches("PHP-").starts_with(version)
        })
        .ok_or_else(|| format!("PHP {} não está no catálogo de pacotes.", version))?;
    install_package_internal(
        Some(app),
        root,
        &package.name,
        &package.url,
        &package.category,
    )?;
    let Some(php_dir) = php_runtime_dir_for_version(root, version) else {
        return Err(format!(
            "PHP {} foi instalado, mas não foi localizado em bin/php.",
            version
        ));
    };
    ensure_php_runtime_dependencies(app, root, version, &php_dir)?;
    validate_php_runtime(&php_dir)
        .map_err(|err| format!("PHP {} foi instalado, mas não executa: {}", version, err))?;
    Ok(())
}

fn ensure_php_runtime_dependencies(
    app: &tauri::AppHandle,
    root: &Path,
    version: &str,
    php_dir: &Path,
) -> Result<(), String> {
    let runtime_tag = detect_php_runtime_tag(php_dir);
    if runtime_tag == "vc11" && !has_vc110_runtime() {
        emit_install_progress(
            app,
            &format!("PHP {}", version),
            "runtime",
            "Instalando dependencias VC++ 2012 (MSVCR110)",
            82,
            "running",
        );
        append_app_log(
            root,
            "Runtime VC11 detected without MSVCR110.dll. Installing Visual C++ 2012 Runtime...",
        )?;
        install_vc2012_runtime(root)?;
        if !has_vc110_runtime() {
            return Err(
                "MSVCR110.dll ainda não disponível após tentativa automática. Instale Visual C++ 2012 Redistributable (x64) e tente novamente."
                    .to_string(),
            );
        }
        emit_install_progress(
            app,
            &format!("PHP {}", version),
            "runtime",
            "Runtime VC++ pronto",
            92,
            "running",
        );
    }
    if matches!(runtime_tag.as_str(), "vc14" | "vc15" | "vs16" | "vs17") && !has_vc14plus_runtime()
    {
        emit_install_progress(
            app,
            &format!("PHP {}", version),
            "runtime",
            "Instalando dependencias VC++ 2015-2022",
            82,
            "running",
        );
        append_app_log(root, "Runtime VC14+/VS detected without vcruntime140. Installing Visual C++ 2015-2022 Runtime...")?;
        install_vc14plus_runtime(root)?;
        if !has_vc14plus_runtime() {
            return Err(
                "vcruntime140.dll ainda não disponível após tentativa automática. Instale Visual C++ 2015-2022 Redistributable (x64) e tente novamente."
                    .to_string(),
            );
        }
        emit_install_progress(
            app,
            &format!("PHP {}", version),
            "runtime",
            "Runtime VC++ pronto",
            92,
            "running",
        );
    }

    let first_check = validate_php_runtime(php_dir);
    if first_check.is_ok() {
        return Ok(());
    }
    let detail = first_check.err().unwrap_or_default();
    let lower = detail.to_ascii_lowercase();

    if lower.contains("msvcr110.dll") || lower.contains("vc11") {
        emit_install_progress(
            app,
            &format!("PHP {}", version),
            "runtime",
            "Instalando dependencias VC++ 2012 (MSVCR110)",
            82,
            "running",
        );
        append_app_log(
            root,
            "MSVCR110.dll dependency detected. Trying to install Visual C++ 2012 Runtime...",
        )?;
        install_vc2012_runtime(root)?;
        let recheck = validate_php_runtime(php_dir);
        if recheck.is_err() {
            let after = recheck
                .err()
                .unwrap_or_else(|| "runtime VC++ 2012 ainda indisponivel".to_string());
            return Err(format!(
                "Dependência do PHP não resolvida automaticamente. Instale o Visual C++ 2012 Redistributable (x64) e tente novamente. Detalhe: {}",
                after
            ));
        }
        emit_install_progress(
            app,
            &format!("PHP {}", version),
            "runtime",
            "Runtime VC++ pronto",
            92,
            "running",
        );
    }
    Ok(())
}

fn install_vc2012_runtime(root: &Path) -> Result<(), String> {
    let local_candidates = [
        root.join("dependencias").join("vcredist_x64_2012.exe"),
        root.join("dependencias").join("vcredist_x64.exe"),
        workspace_root()
            .join("dependencias")
            .join("vcredist_x64_2012.exe"),
        workspace_root()
            .join("dependencias")
            .join("vcredist_x64.exe"),
        root.join("_up_")
            .join("dependencias")
            .join("vcredist_x64_2012.exe"),
        root.join("_up_")
            .join("dependencias")
            .join("vcredist_x64.exe"),
    ];
    if let Some(local_installer) = local_candidates.iter().find(|path| path.exists()) {
        return run_elevated_installer(root, local_installer, "/quiet /norestart");
    }
    Err("Instalador VC++ 2012 não encontrado. Coloque vcredist_x64_2012.exe em dependencias para concluir o PHP 5.6.".to_string())
}

fn install_vc14plus_runtime(root: &Path) -> Result<(), String> {
    let local_candidates = [
        root.join("dependencias").join("VC_redist.x64.exe"),
        root.join("dependencias").join("vcredist_x64_2015_2022.exe"),
        root.join("dependencias").join("vcredist_x64_2019.exe"),
        workspace_root()
            .join("dependencias")
            .join("VC_redist.x64.exe"),
        workspace_root()
            .join("dependencias")
            .join("vcredist_x64_2015_2022.exe"),
        workspace_root()
            .join("dependencias")
            .join("vcredist_x64_2019.exe"),
        root.join("_up_")
            .join("dependencias")
            .join("VC_redist.x64.exe"),
        root.join("_up_")
            .join("dependencias")
            .join("vcredist_x64_2015_2022.exe"),
        root.join("_up_")
            .join("dependencias")
            .join("vcredist_x64_2019.exe"),
    ];
    if let Some(local_installer) = local_candidates.iter().find(|path| path.exists()) {
        return run_elevated_installer(root, local_installer, "/quiet /norestart");
    }
    Err(
        "Instalador VC++ 2015-2022 não encontrado. Coloque VC_redist.x64.exe em dependencias."
            .to_string(),
    )
}

fn run_elevated_installer(root: &Path, installer: &Path, args: &str) -> Result<(), String> {
    let script_dir = root.join("tmp").join("elevated");
    fs::create_dir_all(&script_dir).map_err(|e| e.to_string())?;
    let script = script_dir.join("install-runtime.ps1");
    let result = script_dir.join("install-runtime-result.txt");
    if result.exists() {
        let _ = fs::remove_file(&result);
    }

    let script_content = format!(
        "$ErrorActionPreference = 'Stop'\n$exe = '{}'\nif (-not (Test-Path -LiteralPath $exe)) {{ 'MISSING' | Set-Content -LiteralPath '{}' ; exit 3 }}\n$p = Start-Process -FilePath $exe -ArgumentList '{}' -Wait -PassThru -WindowStyle Hidden\n$p.ExitCode | Set-Content -LiteralPath '{}'\nif ($p.ExitCode -ne 0) {{ exit $p.ExitCode }}\n",
        display_path(installer).replace('\'', "''"),
        display_path(&result).replace('\'', "''"),
        args.replace('\'', "''"),
        display_path(&result).replace('\'', "''")
    );
    fs::write(&script, script_content).map_err(|e| e.to_string())?;

    let launcher = format!(
        "Start-Process -FilePath powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\"'",
        display_path(&script)
    );
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &launcher,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("Não foi possível solicitar elevação para instalar runtime VC++.".to_string());
    }
    if !result.exists() {
        return Err(
            "Instalação do runtime VC++ não confirmou resultado (UAC cancelado?).".to_string(),
        );
    }
    let code_raw = fs::read_to_string(&result).unwrap_or_default();
    let code = code_raw.trim().parse::<i32>().unwrap_or(-1);
    if code == 0 || code == 1638 {
        return Ok(());
    }
    Err(format!(
        "Instalador do runtime VC++ retornou codigo {}.",
        code
    ))
}

fn php_version_installed(root: &Path, version: &str) -> bool {
    php_runtime_dir_for_version(root, version).is_some()
}

fn php_runtime_dir_for_version(root: &Path, version: &str) -> Option<PathBuf> {
    let parent = root.join("bin").join("php");
    fs::read_dir(parent)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains(version))
                && path.join("php.exe").exists()
                && validate_php_runtime(path).is_ok()
        })
}

fn active_php_runtime_dir(root: &Path) -> Option<PathBuf> {
    let parent = root.join("bin").join("php");
    let mut dirs = fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("php.exe").exists())
        .filter(|path| validate_php_runtime(path).is_ok())
        .collect::<Vec<_>>();
    dirs.sort();
    dirs.pop()
}

fn validate_php_runtime(php_dir: &Path) -> Result<(), String> {
    let php = php_dir.join("php.exe");
    if !php.exists() {
        return Err("php.exe ausente".to_string());
    }
    if runtime_requires_vc110(php_dir) && !has_vc110_runtime() {
        return Err(
            "MSVCR110.dll ausente (Visual C++ 2012 x64 runtime não encontrado)".to_string(),
        );
    }
    let mut command = Command::new(&php);
    command.arg("-v");
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "dependencias do runtime ausentes (ex.: VC++/MSVCR)".to_string()
    };
    Err(detail)
}

fn runtime_requires_vc110(php_dir: &Path) -> bool {
    detect_php_runtime_tag(php_dir) == "vc11"
}

fn detect_php_runtime_tag(php_dir: &Path) -> String {
    let folder = php_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if folder.contains("vc11") {
        "vc11".to_string()
    } else if folder.contains("vc14") {
        "vc14".to_string()
    } else if folder.contains("vc15") {
        "vc15".to_string()
    } else if folder.contains("vs16") {
        "vs16".to_string()
    } else if folder.contains("vs17") {
        "vs17".to_string()
    } else {
        String::new()
    }
}

fn has_vc110_runtime() -> bool {
    let win = env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    Path::new(&win)
        .join("System32")
        .join("msvcr110.dll")
        .exists()
        || Path::new(&win)
            .join("SysWOW64")
            .join("msvcr110.dll")
            .exists()
}

fn has_vc14plus_runtime() -> bool {
    let win = env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    (Path::new(&win)
        .join("System32")
        .join("vcruntime140.dll")
        .exists()
        && Path::new(&win)
            .join("System32")
            .join("msvcp140.dll")
            .exists())
        || (Path::new(&win)
            .join("SysWOW64")
            .join("vcruntime140.dll")
            .exists()
            && Path::new(&win)
                .join("SysWOW64")
                .join("msvcp140.dll")
                .exists())
}

fn project_config_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".ipeenv.json")
}

fn read_project_config(project_dir: &Path) -> Result<ProjectConfig, String> {
    let raw = fs::read_to_string(project_config_path(project_dir)).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

fn write_project_config(project_dir: &Path, config: &ProjectConfig) -> Result<(), String> {
    fs::write(
        project_config_path(project_dir),
        serde_json::to_string_pretty(config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn php_cgi_port(version: &str) -> u16 {
    let mut hash = 0u16;
    for byte in version.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u16);
    }
    9100 + (hash % 500)
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn package_catalog_path(root: &Path) -> PathBuf {
    root.join(PACKAGE_CATALOG_PATH)
}

fn site_template_catalog_path(root: &Path) -> PathBuf {
    root.join(SITE_TEMPLATE_CATALOG_PATH)
}

fn mirror_bundled_resources(app: &tauri::AppHandle, root: &Path) -> Result<(), String> {
    let Ok(resource_dir) = app.path().resource_dir() else {
        return Ok(());
    };
    if !is_ephemeral_resource_dir(root, &resource_dir) {
        return Ok(());
    }

    for relative in [PACKAGE_CATALOG_PATH, SITE_TEMPLATE_CATALOG_PATH] {
        let bundled = resource_dir.join(relative);
        let target = root.join(relative);
        if bundled.exists() && !target.exists() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(&bundled, &target).map_err(|e| {
                format!(
                    "Não foi possível copiar {} para {}: {}",
                    relative,
                    display_path(&target),
                    e
                )
            })?;
        }
    }

    Ok(())
}

fn cleanup_bundled_resource_dir(app: &tauri::AppHandle, root: &Path) -> Result<(), String> {
    let Ok(resource_dir) = app.path().resource_dir() else {
        return Ok(());
    };
    if !is_ephemeral_resource_dir(root, &resource_dir) {
        return Ok(());
    }

    if !package_catalog_path(root).exists() || !site_template_catalog_path(root).exists() {
        return Ok(());
    }

    match fs::remove_dir_all(&resource_dir) {
        Ok(()) => append_app_log(root, "_up_ removed after mirroring local resources"),
        Err(err) => append_app_log(root, &format!("Could not remove _up_: {}", err)),
    }
}

fn is_ephemeral_resource_dir(root: &Path, resource_dir: &Path) -> bool {
    resource_dir.file_name().is_some_and(|name| name == "_up_")
        && resource_dir.parent().is_some_and(|parent| parent == root)
}

fn seed_package_catalog(app: &tauri::AppHandle, root: &Path) -> Result<(), String> {
    let target = package_catalog_path(root);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let legacy_local = root.join("config").join("packages.conf");
    if legacy_local.exists() {
        let _ = fs::remove_file(&legacy_local); // Limpa o cache antigo
    }
    let source = bundled_or_workspace_file(app, PACKAGE_CATALOG_PATH);
    let source = if source.exists() {
        source
    } else {
        bundled_or_workspace_file(app, LEGACY_PACKAGE_CATALOG_PATH)
    };
    fs::copy(&source, &target).map_err(|e| {
        format!(
            "Não foi possível preparar catálogo de pacotes a partir de {}: {}",
            display_path(&source),
            e
        )
    })?;
    Ok(())
}

fn seed_site_template_catalog(app: &tauri::AppHandle, root: &Path) -> Result<(), String> {
    let target = site_template_catalog_path(root);
    if target.exists() && fs::metadata(&target).map(|m| m.len()).unwrap_or(0) > 0 {
        return Ok(());
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let legacy_local = root.join("config").join("sites.conf");
    if legacy_local.exists() && fs::metadata(&legacy_local).map(|m| m.len()).unwrap_or(0) > 0 {
        fs::copy(&legacy_local, &target).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let source = bundled_or_workspace_file(app, SITE_TEMPLATE_CATALOG_PATH);
    let source = if source.exists() {
        source
    } else {
        bundled_or_workspace_file(app, LEGACY_SITE_TEMPLATE_CATALOG_PATH)
    };
    if source.exists() && fs::metadata(&source).map(|m| m.len()).unwrap_or(0) > 0 {
        fs::copy(&source, &target).map_err(|e| {
            format!(
                "Não foi possível preparar catálogo de modelos a partir de {}: {}",
                display_path(&source),
                e
            )
        })?;
    } else {
        fs::write(&target, default_site_template_catalog()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn parse_package_catalog(raw: &str) -> Vec<PackageEntry> {
    let mut category = "Geral".to_string();
    let mut entries = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "---" {
            continue;
        }
        if trimmed.starts_with('#') {
            let label = trimmed.trim_start_matches('#').trim();
            if !label.is_empty() 
                && !label.starts_with("http") 
                && !label.contains("After download")
                && !label.contains("NTS =") 
                && !label.contains("Menu >") 
            {
                category = label.to_string();
            }
            continue;
        }
        let Some((name, url)) = trimmed.split_once('=') else {
            continue;
        };
        let preferred = name.starts_with('*');
        entries.push(PackageEntry {
            name: name.trim_start_matches('*').trim().to_string(),
            url: url.trim().to_string(),
            category: category.clone(),
            preferred,
            install_dir: String::new(),
            installed: false,
        });
    }

    entries
}

fn parse_site_template_catalog(raw: &str) -> Vec<SiteTemplate> {
    let mut category = "Sites".to_string();
    let mut entries = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "---" {
            continue;
        }
        if trimmed.starts_with('#') {
            let label = trimmed.trim_start_matches('#').trim();
            if !label.is_empty() {
                category = label.to_string();
            }
            continue;
        }
        let Some((name, source)) = trimmed.rsplit_once('=') else {
            continue;
        };
        let preferred = name.starts_with('*');
        let (clean_name, framework_meta, version_meta, php_min_meta, php_max_meta) =
            parse_site_template_left(name.trim_start_matches('*').trim());
        let (source, php_min_source, php_max_source) = split_template_source(source.trim());
        let (fallback_framework, fallback_version) = split_template_name(&clean_name);
        entries.push(SiteTemplate {
            name: clean_name,
            framework: framework_meta.unwrap_or(fallback_framework),
            version: version_meta.unwrap_or(fallback_version),
            source,
            category: category.clone(),
            php_min: php_min_meta.or(php_min_source),
            php_max: php_max_meta.or(php_max_source),
            preferred,
        });
    }

    if entries.is_empty() {
        parse_site_template_catalog(default_site_template_catalog())
    } else {
        entries
    }
}

fn default_site_template_catalog() -> &'static str {
    "# Sites\nBlank|framework=Blank|version=Padrão|php>=5.6=\nPHP|framework=PHP|version=Padrão|php>=5.6=inline:php\n---\n# Frameworks\nLaravel 12|framework=Laravel|version=12.x|php>=8.2=composer create-project laravel/laravel:^12.0 {name}\nLaravel 11|framework=Laravel|version=11.x|php>=8.2=composer create-project laravel/laravel:^11.0 {name}\nLaravel 10|framework=Laravel|version=10.x|php>=8.1=composer create-project laravel/laravel:^10.0 {name}\nSymfony 7.3|framework=Symfony|version=7.3|php>=8.2=composer create-project symfony/skeleton:^7.3 {name}\nSymfony 6.4 LTS|framework=Symfony|version=6.4 LTS|php>=8.1=composer create-project symfony/skeleton:^6.4 {name}\nCakePHP 5.2|framework=CakePHP|version=5.2|php>=8.1=composer create-project cakephp/app:^5.2 {name}\nCakePHP 5.1|framework=CakePHP|version=5.1|php>=8.1=composer create-project cakephp/app:^5.1 {name}\nCakePHP 5.0|framework=CakePHP|version=5.0|php>=8.1=composer create-project cakephp/app:^5.0 {name}\nCakePHP 4.6|framework=CakePHP|version=4.6|php>=7.4=composer create-project cakephp/app:^4.6 {name}\nCakePHP 4.5|framework=CakePHP|version=4.5|php>=7.4=composer create-project cakephp/app:^4.5 {name}\nCakePHP 3.10|framework=CakePHP|version=3.10|php>=5.6|php<8.0=composer create-project cakephp/app:^3.10 {name}\n---\n# CMS\nWordPress 6.8|framework=WordPress|version=6.8|php>=7.2=https://wordpress.org/latest.zip\n"
}

fn parse_site_template_left(
    left: &str,
) -> (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let mut parts = left.split('|');
    let name = parts.next().unwrap_or("").trim().to_string();
    let mut framework = None;
    let mut version = None;
    let mut php_min = None;
    let mut php_max = None;

    for part in parts {
        let trimmed = part.trim();
        if let Some(value) = trimmed.strip_prefix("framework=") {
            framework = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("version=") {
            version = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("php>=") {
            php_min = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("php<=") {
            php_max = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("php<") {
            php_max = Some(value.trim().to_string());
        }
    }

    (name, framework, version, php_min, php_max)
}

fn split_template_name(name: &str) -> (String, String) {
    if let Some((framework, version)) = name.rsplit_once('-') {
        if version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return (framework.to_string(), version.to_string());
        }
    }
    (name.to_string(), "Padrão".to_string())
}

fn split_template_source(source: &str) -> (String, Option<String>, Option<String>) {
    let mut parts = source.split('|');
    let command = parts.next().unwrap_or("").trim().to_string();
    let mut php_min = None;
    let mut php_max = None;
    for part in parts {
        let trimmed = part.trim();
        if let Some(value) = trimmed.strip_prefix("php>=") {
            php_min = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("php<=") {
            php_max = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("php<") {
            php_max = Some(value.trim().to_string());
        }
    }
    (command, php_min, php_max)
}

struct ToolSpec {
    id: &'static str,
    name: &'static str,
    kind: &'static str,
    source_path: PathBuf,
    source_url: Option<String>,
    install_path: PathBuf,
    executable_path: PathBuf,
}

fn tool_specs(root: &Path) -> Vec<ToolSpec> {
    let deps = root.join("dependencias");
    let urls = tool_source_urls(root);
    tool_specs_with_deps(root, &deps, &urls)
}

fn tool_source_urls(root: &Path) -> HashMap<String, String> {
    let candidates = [
        package_catalog_path(root),
        workspace_root().join(PACKAGE_CATALOG_PATH),
        workspace_root().join(LEGACY_PACKAGE_CATALOG_PATH),
    ];

    for path in candidates {
        if !path.exists() {
            continue;
        }
        if let Ok(raw) = fs::read_to_string(path) {
            let mut urls = HashMap::new();
            for entry in parse_package_catalog(&raw) {
                urls.insert(slugify(&entry.name), entry.url);
            }
            return urls;
        }
    }

    HashMap::new()
}

fn bundled_tool_specs(app: &tauri::AppHandle, root: &Path) -> Vec<ToolSpec> {
    let deps = dependency_root(app, root);
    let urls = tool_source_urls(root);
    tool_specs_with_deps(root, &deps, &urls)
}

fn tool_specs_with_deps(
    root: &Path,
    deps: &Path,
    urls: &HashMap<String, String>,
) -> Vec<ToolSpec> {
    let bin = root.join("bin");
    vec![
        ToolSpec {
            id: "cmder",
            name: "Cmder",
            kind: "Terminal",
            source_path: deps.join("cmder.zip"),
            source_url: urls.get("cmder").cloned(),
            install_path: bin.join("cmder"),
            executable_path: bin.join("cmder").join("Cmder.exe"),
        },
        ToolSpec {
            id: "notepadpp",
            name: "Notepad++",
            kind: "Editor",
            source_path: deps.join("npp.8.9.6.portable.x64.zip"),
            source_url: urls.get("notepad").cloned(),
            install_path: bin.join("notepadpp"),
            executable_path: bin.join("notepadpp").join("notepad++.exe"),
        },
        ToolSpec {
            id: "heidisql",
            name: "HeidiSQL",
            kind: "Banco de dados",
            source_path: deps.join("HeidiSQL_12.17_64_Portable.zip"),
            source_url: urls.get("heidisql").cloned(),
            install_path: bin.join("heidisql"),
            executable_path: bin.join("heidisql").join("heidisql.exe"),
        },
        ToolSpec {
            id: "ngrok",
            name: "ngrok",
            kind: "Tunel",
            source_path: deps.join("ngrok-v3-stable-windows-amd64.zip"),
            source_url: None,
            install_path: bin.join("ngrok"),
            executable_path: bin.join("ngrok").join("ngrok.exe"),
        },
    ]
}

fn tool_executable(root: &Path, id: &str) -> Option<PathBuf> {
    tool_specs(root)
        .into_iter()
        .find(|spec| spec.id == id && spec.executable_path.exists())
        .map(|spec| spec.executable_path)
}

fn bundled_or_workspace_file(app: &tauri::AppHandle, relative: &str) -> PathBuf {
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join(relative);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join(relative);
        if candidate.exists() {
            return candidate;
        }
    }
    workspace_root().join(relative)
}

fn dependency_root(app: &tauri::AppHandle, root: &Path) -> PathBuf {
    let local = root.join("dependencias");
    if local.exists() {
        return local;
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled = resource_dir.join("dependencias");
        if bundled.exists() {
            return bundled;
        }
    }

    workspace_root().join("dependencias")
}

fn install_bundled_portable_tools(app: &tauri::AppHandle, root: &Path) -> Result<(), String> {
    for spec in bundled_tool_specs(app, root) {
        if spec.executable_path.exists() || !spec.source_path.exists() {
            continue;
        }
        fs::create_dir_all(&spec.install_path).map_err(|e| e.to_string())?;
        let result = if spec.id == "heidisql" {
            install_heidisql_isolated(&spec)
        } else {
            expand_zip(&spec.source_path, &spec.install_path)
                .and_then(|_| flatten_single_directory(&spec.install_path))
        };

        match result {
            Ok(()) => append_app_log(root, &format!("Tool prepared in bin: {}", spec.name))?,
            Err(err) => append_app_log(root, &format!("Failed to prepare {}: {}", spec.name, err))?,
        }
    }
    Ok(())
}

fn install_heidisql_isolated(spec: &ToolSpec) -> Result<(), String> {
    install_heidisql_isolated_with_progress(spec, |_| {})
}

fn install_heidisql_isolated_with_progress<F>(spec: &ToolSpec, on_progress: F) -> Result<(), String>
where
    F: FnMut(u8),
{
    if !spec.source_path.exists() {
        return Err(format!(
            "Dependência não encontrada: {}",
            display_path(&spec.source_path)
        ));
    }
    fs::create_dir_all(&spec.install_path).map_err(|e| e.to_string())?;
    if spec
        .source_path
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        extract_zip_with_progress(&spec.source_path, &spec.install_path, on_progress)?;
        flatten_single_directory(&spec.install_path)
    } else {
        Err("Use a versão portable do HeidiSQL em dependencias para manter isolamento.".to_string())
    }
}

fn download_file(url: &str, target: &Path) -> Result<(), String> {
    download_file_with_progress(url, target, |_| {})
}

fn download_file_with_progress<F>(
    url: &str,
    target: &Path,
    mut on_progress: F,
) -> Result<(), String>
where
    F: FnMut(u8),
{
    if url.trim().is_empty() {
        return Err("URL do pacote está vazia no catálogo de pacotes.".to_string());
    }
    on_progress(15);
    let client = reqwest::blocking::Client::builder()
        .user_agent("Ipeenv/0.1 QuickAdd")
        .build()
        .map_err(|e| e.to_string())?;
    let mut attempted_url = url.to_string();
    let mut response = client
        .get(url)
        .send()
        .map_err(|e| format!("Falha ao baixar {}: {}", url, e))?;
    if !response.status().is_success() {
        if let Some(fallback) = mysql_download_fallback(url) {
            attempted_url = fallback.clone();
            response = client
                .get(&fallback)
                .send()
                .map_err(|e| format!("Falha ao baixar {}: {}", fallback, e))?;
        }
    }
    if !response.status().is_success() {
        return Err(format!(
            "Download retornou status {} para {}",
            response.status(),
            attempted_url
        ));
    }
    on_progress(20);
    let mut file = File::create(target).map_err(|e| e.to_string())?;
    let total = response.content_length();
    let mut downloaded = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = response.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
        downloaded += read as u64;
        if let Some(total) = total.filter(|value| *value > 0) {
            let percent = 20 + ((downloaded.saturating_mul(40) / total).min(40) as u8);
            on_progress(percent);
        }
    }
    on_progress(60);
    Ok(())
}

fn mysql_download_fallback(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("https://dev.mysql.com/get/")
        .or_else(|| url.strip_prefix("https://cdn.mysql.com/"))?;

    let parts = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
    if parts.len() >= 3
        && parts[0].eq_ignore_ascii_case("Downloads")
        && parts[1].starts_with("MySQL-")
    {
        let family = parts[1].to_ascii_lowercase();
        let file = parts.last()?;
        return Some(format!(
            "https://cdn.mysql.com/archives/{}/{}",
            family, file
        ));
    }

    None
}

fn expand_zip(source: &Path, target: &Path) -> Result<(), String> {
    extract_zip(source, target)
}

fn expand_zip_with_progress<F>(source: &Path, target: &Path, on_progress: F) -> Result<(), String>
where
    F: FnMut(u8),
{
    extract_zip_with_progress(source, target, on_progress)
}

fn extract_zip(source: &Path, target: &Path) -> Result<(), String> {
    extract_zip_with_progress(source, target, |_| {})
}

fn extract_zip_with_progress<F>(
    source: &Path,
    target: &Path,
    mut on_progress: F,
) -> Result<(), String>
where
    F: FnMut(u8),
{
    let file = File::open(source).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let total = archive.len().max(1);
    on_progress(62);

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        let output = target.join(enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut output_file = File::create(&output).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut output_file).map_err(|e| e.to_string())?;
        }
        let percent = 62 + ((((index + 1) as u64 * 26) / total as u64).min(26) as u8);
        on_progress(percent);
    }

    on_progress(88);
    Ok(())
}

fn package_extension(url: &str) -> &'static str {
    let lower = url.to_ascii_lowercase();
    if lower.contains(".tar.xz") {
        ".tar.xz"
    } else if lower.contains(".zip") {
        ".zip"
    } else if lower.contains(".exe") {
        ".exe"
    } else {
        ".download"
    }
}

fn validate_port(port: u16, label: &str) -> Result<(), String> {
    if port == 0 {
        Err(format!("Porta {} invalida.", label))
    } else {
        Ok(())
    }
}

fn package_target_dir(root: &Path, name: &str, category: &str, url: &str) -> PathBuf {
    let lower = name.to_ascii_lowercase();
    let lower_category = category.to_ascii_lowercase();
    let folder = package_folder_name(name, url);
    if lower.starts_with("php-") {
        root.join("bin").join("php").join(folder)
    } else if lower.starts_with("apache") {
        root.join("bin").join("apache").join(folder)
    } else if lower.starts_with("mysql") {
        root.join("bin").join("mysql").join(folder)
    } else if lower.starts_with("nginx") {
        root.join("bin").join("nginx").join(folder)
    } else if lower.starts_with("node") {
        root.join("bin").join("node").join(folder)
    } else if lower.starts_with("postgresql") {
        root.join("bin").join("postgresql").join(folder)
    } else if lower.starts_with("go-") {
        root.join("bin").join("go").join(folder)
    } else if lower == "phpmyadmin" || lower.starts_with("phpmyadmin-") {
        root.join("www").join("phpmyadmin")
    } else if lower_category.contains("db tools") || lower == "dbeaver" {
        root.join("bin").join(slugify(name)).join(folder)
    } else {
        root.join("bin").join(slugify(name)).join(folder)
    }
}

fn package_folder_name(name: &str, url: &str) -> String {
    let file_name = url
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .trim_end_matches(".zip")
        .trim_end_matches(".msi")
        .trim_end_matches(".exe")
        .trim_end_matches(".tar.xz");
    if file_name.is_empty() {
        slugify(name)
    } else {
        file_name.to_string()
    }
}

fn package_is_installed(target: &Path, name: &str) -> bool {
    if !target.exists() {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("php-") {
        target.join("php.exe").exists()
    } else if lower.starts_with("apache") {
        target.join("bin").join("httpd.exe").exists() || target.join("httpd.exe").exists()
    } else if lower.starts_with("mysql") {
        target.join("bin").join("mysqld.exe").exists() || target.join("mysqld.exe").exists()
    } else {
        fs::read_dir(target)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false)
    }
}

fn compare_versions_desc(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |value: &str| {
        value
            .split('.')
            .map(|part| part.parse::<u32>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    parse(b).cmp(&parse(a))
}

fn active_runtime_dir(parent: &Path, executable_name: &str) -> Option<PathBuf> {
    let mut dirs = fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.join(executable_name).exists() || path.join("bin").join(executable_name).exists()
        })
        .collect::<Vec<_>>();
    dirs.sort();
    dirs.pop()
}

fn flatten_named_child(target: &Path, child_name: &str) -> Result<(), String> {
    let child = target.join(child_name);
    if child.is_dir() {
        copy_dir_contents(&child, target)?;
    }
    Ok(())
}

fn flatten_single_directory(target: &Path) -> Result<(), String> {
    let entries = fs::read_dir(target)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    if entries.len() == 1 && entries[0].path().is_dir() {
        copy_dir_contents(&entries[0].path(), target)?;
    }
    Ok(())
}

fn copy_dir_contents(source: &Path, target: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            fs::create_dir_all(&target_path).map_err(|e| e.to_string())?;
            copy_dir_contents(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
fn allow_firewall(app: tauri::AppHandle) -> Result<ActionResult, String> {
    let root = app_root(&app)?;
    let candidates = firewall_candidates(&root);

    let mut created: Vec<String> = Vec::new();
    let mut existing: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for (name, exe) in &candidates {
        if !exe.exists() {
            skipped.push(format!("{} (não instalado)", name));
            continue;
        }
        if firewall_rule_exists(name, exe) {
            existing.push(name.clone());
            continue;
        }
        match add_firewall_rule(name, exe) {
            Ok(()) => {
                created.push(name.clone());
                append_app_log(&root, &format!("Firewall: rule created for {}", name))?;
            }
            Err(err) => errors.push(format!("{}: {}", name, err)),
        }
    }

    if !errors.is_empty() {
        request_elevated_firewall_rules(&root, &candidates)?;
        append_app_log(&root, "Elevation requested for firewall rules")?;
        return Ok(ActionResult::coded(
            true,
            "firewall.elevationRequested",
            "Permissão elevada solicitada para liberar Apache, MySQL e PHP-CGI no Firewall. Confirme o UAC.",
            [],
        ));
    }

    let mut parts: Vec<String> = Vec::new();
    if !created.is_empty() {
        parts.push(format!("Regras criadas: {}", created.join(", ")));
    }
    if !existing.is_empty() {
        parts.push(format!("Ja existiam: {}", existing.join(", ")));
    }
    if !skipped.is_empty() {
        parts.push(format!("Ignorados: {}", skipped.join(", ")));
    }
    if !errors.is_empty() {
        parts.push(format!("Erros: {}", errors.join("; ")));
    }

    Ok(ActionResult::message(
        errors.is_empty(),
        if parts.is_empty() {
            "Nenhum executavel encontrado para liberar.".to_string()
        } else {
            parts.join(" | ")
        },
    ))
}

fn firewall_candidates(root: &Path) -> Vec<(String, PathBuf)> {
    let bin = root.join("bin");
    let mut candidates = Vec::new();

    let apache_exe = active_runtime_dir(&bin.join("apache"), "httpd.exe")
        .unwrap_or_else(|| bin.join("apache"))
        .join("bin")
        .join("httpd.exe");
    candidates.push(("Ipeenv Apache".to_string(), apache_exe));

    let mysql_exe = active_runtime_dir(&bin.join("mysql"), "mysqld.exe")
        .unwrap_or_else(|| bin.join("mysql"))
        .join("bin")
        .join("mysqld.exe");
    candidates.push(("Ipeenv MySQL".to_string(), mysql_exe));

    if let Ok(entries) = fs::read_dir(bin.join("php")) {
        for entry in entries.filter_map(Result::ok) {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let version = dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let php_cgi = dir.join("php-cgi.exe");
            let php_exe = dir.join("php.exe");
            if php_cgi.exists() {
                candidates.push((format!("Ipeenv PHP-CGI {}", version), php_cgi));
            }
            if php_exe.exists() {
                candidates.push((format!("Ipeenv PHP {}", version), php_exe));
            }
        }
    }

    candidates
}

fn firewall_rule_exists(name: &str, exe: &Path) -> bool {
    if !cfg!(target_os = "windows") {
        return true;
    }
    let escaped_name = name.replace('\'', "''");
    let script = format!(
        "$rules = Get-NetFirewallRule -DisplayName '{}' -ErrorAction SilentlyContinue; if (-not $rules) {{ exit 1 }}; foreach ($rule in $rules) {{ Get-NetFirewallApplicationFilter -AssociatedNetFirewallRule $rule | Select-Object -ExpandProperty Program }}",
        escaped_name
    );
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &script,
    ]);
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);

    let Ok(output) = command.output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let expected = exe
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().replace('\\', "/").to_ascii_lowercase())
        .any(|program| program == expected)
}

fn add_firewall_rule(name: &str, exe: &Path) -> Result<(), String> {
    if firewall_rule_exists(name, exe) {
        return Ok(());
    }
    let output = Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &format!("name={}", name),
            "dir=in",
            "action=allow",
            &format!("program={}", exe.display()),
            "enable=yes",
            "profile=any",
        ])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Err(if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        })
    }
}

fn request_elevated_firewall_rules(
    root: &Path,
    candidates: &[(String, PathBuf)],
) -> Result<(), String> {
    let script_dir = root.join("tmp").join("elevated");
    fs::create_dir_all(&script_dir).map_err(|e| e.to_string())?;
    let script = script_dir.join("allow-firewall.ps1");
    let result = script_dir.join("allow-firewall-result.txt");
    let mut content = String::new();
    content.push_str("$ErrorActionPreference = 'Stop'\n");
    content.push_str("$created = @()\n");
    for (name, exe) in candidates {
        if !exe.exists() {
            continue;
        }
        if firewall_rule_exists(name, exe) {
            continue;
        }
        let escaped_name = name.replace('\'', "''");
        let escaped_exe = exe.to_string_lossy().replace('\'', "''");
        content.push_str(&format!(
            "$existing = Get-NetFirewallRule -DisplayName '{}' -ErrorAction SilentlyContinue\nif ($existing) {{ $existing | Remove-NetFirewallRule }}\nNew-NetFirewallRule -DisplayName '{}' -Direction Inbound -Action Allow -Program '{}' -Profile Any -Enabled True | Out-Null\n$created += '{}'\n",
            escaped_name, escaped_name, escaped_exe, escaped_name
        ));
    }
    content.push_str(&format!(
        "'OK: ' + ($created -join ', ') | Set-Content -LiteralPath '{}' -Encoding UTF8\n",
        result.to_string_lossy().replace('\'', "''")
    ));
    fs::write(&script, content).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&result);

    #[cfg(target_os = "windows")]
    {
        let script_path = script.to_string_lossy().replace('"', "\\\"");
        let command = format!(
            "Start-Process -FilePath powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\"'",
            script_path
        );
        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &command,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("Não foi possível solicitar elevação para liberar firewall.".to_string());
        }
    }

    if result.exists() {
        append_app_log(
            root,
            &format!(
                "Elevated firewall: {}",
                fs::read_to_string(&result).unwrap_or_default().trim()
            ),
        )?;
    } else {
        append_app_log(root, "Elevated firewall requested, but no result was written. Permission may have been canceled or the script failed.")?;
    }

    Ok(())
}
