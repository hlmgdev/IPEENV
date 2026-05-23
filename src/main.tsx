import React, { useEffect, useMemo, useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  AlertTriangle,
  Database,
  ExternalLink,
  FileText,
  Folder,
  Globe,
  HardDrive,
  Plus,
  RefreshCw,
  Search,
  Server,
  Settings,
  Shield,
  Square,
  Terminal,
  Wrench,
} from "lucide-react";
import "./styles.css";
import { initialLocale, localeLabels, translate, type Locale } from "./i18n";

type ServiceInfo = {
  id: string;
  name: string;
  version: string;
  port: number | null;
  status: "running" | "stopped";
  pid: number | null;
  executable: string;
  available: boolean;
  port_available: boolean | null;
  last_message: string;
  enabled: boolean;
};

type ProjectInfo = {
  name: string;
  path: string;
  vhost_path: string;
  http_url: string;
  https_url: string;
  domain: string;
  php_version: string | null;
  php_cgi_port: number | null;
  ssl_enabled: boolean;
  host_configured: boolean;
  framework: string;
  modified_at: string;
};

type EnvironmentInfo = {
  app_version: string;
  root_dir: string;
  http_port: number;
  https_port: number;
  mysql_port: number;
  https_enabled: boolean;
  services: ServiceInfo[];
  diagnostics: Array<{ level: string; title: string; message: string }>;
};

type ActionResult = {
  ok: boolean;
  message: string;
  code?: string;
  params?: Record<string, string>;
};

type PackageEntry = {
  name: string;
  url: string;
  category: string;
  preferred: boolean;
  install_dir: string;
  installed: boolean;
};

type SiteTemplate = {
  name: string;
  framework: string;
  version: string;
  source: string;
  category: string;
  php_min: string | null;
  php_max?: string | null;
  preferred: boolean;
};

type PhpOption = {
  version: string;
  label: string;
  installed: boolean;
  installable: boolean;
};

type PhpRuntimeInfo = {
  version: string;
  name: string;
  path: string;
  ini_path: string;
  extension_count: number;
};

type PhpExtensionInfo = {
  name: string;
  dll: string;
  enabled: boolean;
};

type ProjectProgress = {
  project: string;
  step: string;
  percent: number;
  status: string;
};

type InstallProgress = {
  item: string;
  kind: string;
  step: string;
  percent: number;
  status: string;
};

type ToolInfo = {
  id: string;
  name: string;
  kind: string;
  source_path: string;
  install_path: string;
  installed: boolean;
  available_source: boolean;
};

type Section = "overview" | "projects" | "services" | "tools" | "logs" | "settings";

const serviceIcon: Record<string, React.ReactNode> = {
  apache: <Server size={17} />,
  mysql: <Database size={17} />,
  php: <FileText size={17} />,
};

type Translate = (key: string, params?: Record<string, string | number>) => string;

function serviceStatusText(status: ServiceInfo["status"], t: Translate) {
  return status === "running" ? t("Em execução") : t("Parado");
}

function App() {
  const [locale, setLocale] = useState<Locale>(initialLocale);
  const t = useMemo(() => (key: string, params?: Record<string, string | number>) => translate(locale, key, params), [locale]);
  const [section, setSection] = useState<Section>("overview");
  const [env, setEnv] = useState<EnvironmentInfo | null>(null);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [packages, setPackages] = useState<PackageEntry[]>([]);
  const [siteTemplates, setSiteTemplates] = useState<SiteTemplate[]>([]);
  const [phpOptions, setPhpOptions] = useState<PhpOption[]>([]);
  const [phpRuntimes, setPhpRuntimes] = useState<PhpRuntimeInfo[]>([]);
  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [logs, setLogs] = useState(() => t("Carregando logs..."));
  const [activeLog, setActiveLog] = useState("app");
  const [query, setQuery] = useState("");
  const [modalOpen, setModalOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<string>(() => t("Inicializando ambiente local..."));
  const [projectProgress, setProjectProgress] = useState<ProjectProgress | null>(null);
  const [installProgress, setInstallProgress] = useState<InstallProgress | null>(null);

  const refresh = async () => {
    const [info, list] = await Promise.all([
      invoke<EnvironmentInfo>("get_environment_info"),
      invoke<ProjectInfo[]>("list_projects"),
    ]);
    setEnv(info);
    setProjects(list);
    invoke<PackageEntry[]>("list_packages").then(setPackages).catch(() => setPackages([]));
    invoke<SiteTemplate[]>("list_site_templates").then(setSiteTemplates).catch(() => setSiteTemplates([]));
    invoke<PhpOption[]>("list_php_options").then(setPhpOptions).catch(() => setPhpOptions([]));
    invoke<PhpRuntimeInfo[]>("list_php_runtimes").then(setPhpRuntimes).catch(() => setPhpRuntimes([]));
    invoke<ToolInfo[]>("list_local_tools").then(setTools).catch(() => setTools([]));
    setNotice(t("Ambiente sincronizado."));
  };

  useEffect(() => {
    localStorage.setItem("ipeenv:locale", locale);
  }, [locale]);

  useEffect(() => {
    refresh().catch((error) => setNotice(String(error)));
  }, []);

  useEffect(() => {
    if (!modalOpen) setProjectProgress(null);
  }, [modalOpen]);

  useEffect(() => {
    const unlisten = listen<ProjectProgress>("project-progress", (event) => {
      setProjectProgress(event.payload);
    });
    return () => {
      unlisten.then((dispose) => dispose());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<InstallProgress>("install-progress", (event) => {
      setInstallProgress(event.payload);
    });
    return () => {
      unlisten.then((dispose) => dispose());
    };
  }, []);

  useEffect(() => {
    invoke<{ content: string }>("read_logs", { kind: activeLog })
      .then((result) => setLogs(result.content))
      .catch((error) => setLogs(String(error)));
  }, [activeLog, notice]);

  const filteredProjects = useMemo(
    () => projects.filter((project) => project.name.toLowerCase().includes(query.toLowerCase())),
    [projects, query],
  );

  const runningCount = env?.services.filter((service) => service.status === "running").length ?? 0;
  const missingBins = env?.services.filter((service) => !service.available).length ?? 0;
  const actionMessage = (result: ActionResult) =>
    result.code ? t(`action.${result.code}`, result.params) : t(result.message);

  const runAction = async (label: string, action: () => Promise<ActionResult>) => {
    setBusy(label);
    try {
      const result = await action();
      await refresh();
      setNotice(actionMessage(result));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(null);
    }
  };

  const serviceAction = (service: string, command: "start_service" | "stop_service" | "restart_service") => {
    runAction(`${command}:${service}`, () => invoke<ActionResult>(command, { service }));
  };

  const enableAction = (service: string, command: "enable_service" | "disable_service") => {
    runAction(`${command}:${service}`, () => invoke<ActionResult>(command, { service }));
  };

  return (
    <div className="app">
      <header className="titlebar">
        <div className="brand">
          <img src="/assets/logo_arvore.png" alt="" />
          <div>
            <strong>Ipeenv</strong>
            <span> v{env?.app_version ?? "0.1.0"} · {env?.root_dir ?? t("Preparando ambiente")}</span>
          </div>
        </div>
        <div className="window-dots"><span /><span /><span /></div>
      </header>

      <div className="toolbar">
        <button onClick={() => runAction("start:all", async () => {
          for (const service of ["apache", "mysql"]) {
            await invoke<ActionResult>("start_service", { service });
          }
          return { ok: true, message: t("Comando de início enviado para Apache e MySQL.") };
        })}>
          <Server size={15} /> {t("Iniciar ambiente")}
        </button>
        <button onClick={() => runAction("stop:all", async () => {
          for (const service of ["apache", "mysql"]) {
            await invoke<ActionResult>("stop_service", { service });
          }
          return { ok: true, message: t("Serviços parados.") };
        })}>
          <Square size={14} /> {t("Parar")}
        </button>
        <button onClick={() => runAction("config", () => invoke<ActionResult>("generate_apache_config"))}>
          <RefreshCw size={14} /> {t("Gerar configurações")}
        </button>
        <span className="toolbar-sep" />
        <button className="primary" onClick={() => setModalOpen(true)}>
          <Plus size={15} /> {t("Novo projeto")}
        </button>
        <select
          className="language-select"
          aria-label={t("Idioma")}
          value={locale}
          onChange={(event) => setLocale(event.target.value as Locale)}
        >
          {(Object.keys(localeLabels) as Locale[]).map((item) => (
            <option key={item} value={item}>{localeLabels[item]}</option>
          ))}
        </select>
        <div className="search">
          <Search size={14} />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("Buscar projeto")} />
        </div>
      </div>

      <div className="layout">
        <aside className="sidebar">
          <NavItem active={section === "overview"} icon={<Activity size={15} />} label={t("Visão geral")} onClick={() => setSection("overview")} />
          <NavItem active={section === "projects"} icon={<Globe size={15} />} label={t("Projetos")} badge={projects.length} onClick={() => setSection("projects")} />
          <NavItem active={section === "services"} icon={<Server size={15} />} label={t("Serviços")} badge={`${runningCount}/${env?.services.length ?? 3}`} onClick={() => setSection("services")} />
          <NavItem active={section === "tools"} icon={<Wrench size={15} />} label={t("Ferramentas")} badge={packages.length} onClick={() => setSection("tools")} />
          <NavItem active={section === "logs"} icon={<Terminal size={15} />} label={t("Logs")} onClick={() => setSection("logs")} />
          <NavItem active={section === "settings"} icon={<Settings size={15} />} label={t("Preferências")} onClick={() => setSection("settings")} />
          <div className="sidebar-card">
            <span className={missingBins ? "dot warn" : "dot ok"} />
            <div>
              <strong>{missingBins ? t("Binários pendentes") : t("Base pronta")}</strong>
              <small>{missingBins ? t("{{count}} executáveis ainda não existem em bin/", { count: missingBins }) : t("Pastas e configurações criadas")}</small>
            </div>
          </div>
        </aside>

        <main>
          {section === "overview" && (
            <Overview
              t={t}
              env={env}
              projects={projects}
              openProjects={() => setSection("projects")}
              openWww={() => runAction("www", () => invoke<ActionResult>("open_www_folder"))}
              openLogs={() => setSection("logs")}
              serviceAction={serviceAction}
              busy={busy}
              onOpenUrl={(url) => invoke<ActionResult>("open_url", { url }).then((r) => setNotice(actionMessage(r)))}
              onOpenPath={(path) => invoke<ActionResult>("open_path", { path }).then((r) => setNotice(actionMessage(r)))}
            />
          )}
          {section === "projects" && (
            <ProjectsView
              t={t}
              projects={filteredProjects}
              onCreate={() => setModalOpen(true)}
              onOpenWww={() => runAction("www", () => invoke<ActionResult>("open_www_folder"))}
              onOpenUrl={(url) => invoke<ActionResult>("open_url", { url }).then((r) => setNotice(actionMessage(r)))}
              onOpenProject={(domain) => runAction(`open-project:${domain}`, () => invoke<ActionResult>("open_project", { domain }))}
              onOpenPath={(path) => invoke<ActionResult>("open_path", { path }).then((r) => setNotice(actionMessage(r)))}
              onOpenVhost={(domain) => runAction(`vhost:${domain}`, () => invoke<ActionResult>("open_vhost_file", { domain }))}
              onOpenVhostsFolder={() => runAction("vhosts", () => invoke<ActionResult>("open_vhosts_folder"))}
              onSsl={(domain) => runAction(`ssl:${domain}`, () => invoke<ActionResult>("enable_ssl", { domain }))}
              onHost={(domain) => runAction(`host:${domain}`, () => invoke<ActionResult>("add_host", { domain }))}
            />
          )}
          {section === "services" && (
            <ServicesView
              t={t}
              env={env}
              services={env?.services ?? []}
              serviceAction={serviceAction}
              enableAction={enableAction}
              busy={busy}
              onAllowFirewall={() => runAction("firewall", () => invoke<ActionResult>("allow_firewall"))}
              onUpdatePorts={(httpPort, httpsPort, mysqlPort) =>
                runAction("ports", () =>
                  invoke<ActionResult>("update_ports", {
                    ports: {
                      http_port: httpPort,
                      https_port: httpsPort,
                      mysql_port: mysqlPort,
                    },
                  }),
                )
              }
              onUpdateHttps={(enabled) => runAction("https", () => invoke<ActionResult>("update_https", { request: { enabled } }))}
            />
          )}
          {section === "tools" && (
            <ToolsView
              t={t}
              packages={packages}
              tools={tools}
              progress={installProgress}
              busy={busy}
              onOpenPackages={() => runAction("packages", () => invoke<ActionResult>("open_packages_config"))}
              onInstallPackage={(entry) =>
                runAction(`package:${entry.name}`, () =>
                  invoke<ActionResult>("install_package", {
                    name: entry.name,
                    url: entry.url,
                    category: entry.category,
                  }),
                )
              }
              onInstallTool={(tool) => runAction(`install:${tool}`, () => invoke<ActionResult>("install_local_tool", { tool }))}
              onLaunchTool={(tool) => runAction(`launch:${tool}`, () => invoke<ActionResult>("launch_tool", { tool }))}
            />
          )}
          {section === "logs" && (
            <LogsView t={t} active={activeLog} setActive={setActiveLog} logs={logs} />
          )}
          {section === "settings" && (
            <SettingsView
              t={t}
              env={env}
              phpRuntimes={phpRuntimes}
              onOpenHosts={() => runAction("hosts", () => invoke<ActionResult>("open_hosts_file"))}
              onOpenPhpIni={(version) => runAction(`php-ini:${version}`, () => invoke<ActionResult>("open_php_ini", { version }))}
              onTogglePhpExtension={(version, extension, enabled) =>
                runAction(`php-ext:${version}:${extension}`, () =>
                  invoke<ActionResult>("set_php_extension", { version, extension, enabled }),
                )
              }
            />
          )}
        </main>
      </div>

      <footer className="statusbar">
        <span>{notice}</span>
        <span>Apache :{env?.http_port ?? 80} · HTTPS :{env?.https_port ?? 443} · MySQL :{env?.mysql_port ?? 3306}</span>
      </footer>

      {modalOpen && (
        <CreateProjectModal
          t={t}
          templates={siteTemplates}
          phpOptions={phpOptions}
          httpsEnabled={env?.https_enabled ?? true}
          progress={projectProgress}
          onClose={() => setModalOpen(false)}
          onOpenConfig={() => runAction("sites", () => invoke<ActionResult>("open_sites_config"))}
          onCreate={(name, template, phpVersion, addHost, enableSsl) =>
            runAction("create", async () => {
              const result = await invoke<ActionResult>("create_project", { request: { name, template, php_version: phpVersion, add_host: addHost, enable_ssl: enableSsl } });
              if (result.ok) setModalOpen(false);
              return result;
            })
          }
        />
      )}
    </div>
  );
}

function NavItem(props: { active: boolean; icon: React.ReactNode; label: string; badge?: React.ReactNode; onClick: () => void }) {
  return (
    <button className={`nav-item ${props.active ? "active" : ""}`} onClick={props.onClick}>
      {props.icon}
      <span>{props.label}</span>
      {props.badge !== undefined && <em>{props.badge}</em>}
    </button>
  );
}

function Overview(props: {
  t: Translate;
  env: EnvironmentInfo | null;
  projects: ProjectInfo[];
  openProjects: () => void;
  openWww: () => void;
  openLogs: () => void;
  serviceAction: (service: string, command: "start_service" | "stop_service" | "restart_service") => void;
  busy: string | null;
  onOpenUrl: (url: string) => void;
  onOpenPath: (path: string) => void;
}) {
  const { t } = props;
  const services = props.env?.services ?? [];
  const enabledServices = services.filter((service) => service.enabled);
  const running = enabledServices.filter((service) => service.status === "running").length;
  const blocked = enabledServices.filter((service) => service.port_available === false && service.status !== "running").length;

  return (
    <section className="view">
      <div className="view-head">
        <div>
          <h1>{t("Visão geral")}</h1>
          <p>{t("Controle local para Apache, PHP, MySQL, projetos .test e SSL.")}</p>
        </div>
        <button className="btn" onClick={props.openProjects}><Globe size={14} /> {t("Ver projetos")}</button>
      </div>

      <div className="stats">
        <Stat label={t("Serviços ativos")} value={`${running}/${services.length || 3}`} note={t("processos controlados pelo app")} />
        <Stat label={t("Projetos")} value={props.projects.length} note={t("pastas dentro de www")} />
        <Stat label={t("Portas bloqueadas")} value={blocked} note="80, 443 ou 3306" tone={blocked ? "warn" : "ok"} />
        <Stat label={t("Root isolado")} value="OK" note={props.env?.root_dir ?? t("criando...")} />
      </div>

      {props.env?.diagnostics.length ? (
        <div className="diagnostics">
          {props.env.diagnostics.map((item) => (
            <div className={`diagnostic ${item.level}`} key={`${item.title}:${item.message}`}>
              <AlertTriangle size={16} />
              <div><strong>{item.title}</strong><span>{item.message}</span></div>
            </div>
          ))}
        </div>
      ) : null}

      <div className="grid two">
        <Panel title={t("Serviços principais")}>
          <div className="mini-list">
            {enabledServices.map((service) => (
              <div className="mini-row" key={service.id}>
                <span className={`dot ${service.status === "running" ? "ok" : service.available ? "off" : "warn"}`} />
                <strong>{service.name}</strong>
                <small>{service.port ? `:${service.port}` : service.version}</small>
                <label
                  className="switch"
                  title={service.status === "running" ? t("Parar") : t("Iniciar")}
                >
                  <input
                    type="checkbox"
                  disabled={props.busy?.includes(service.id)}
                    checked={service.status === "running"}
                    onChange={() => props.serviceAction(service.id, service.status === "running" ? "stop_service" : "start_service")}
                  />
                  <span />
                </label>
              </div>
            ))}
          </div>
        </Panel>
        <Panel title={t("Projetos")}>
          <div className="mini-list" style={{ maxHeight: '240px', overflowY: 'auto' }}>
            {props.projects.map((project) => (
              <div className="mini-row" key={project.name}>
                <span className={`dot ${project.ssl_enabled ? "ok" : "off"}`} />
                <strong>{project.name}</strong>
                <small>{project.domain}</small>
                <div style={{ display: 'flex', gap: '4px' }}>
                  <button className="icon-button" onClick={() => props.onOpenUrl(`http${project.ssl_enabled ? 's' : ''}://${project.domain}`)} title={t("Abrir no navegador")}><Globe size={13} /></button>
                  <button className="icon-button" onClick={() => props.onOpenPath(project.path)} title={t("Abrir pasta")}><Folder size={13} /></button>
                </div>
              </div>
            ))}
            {!props.projects.length && <EmptyLine text={t("Nenhum projeto ainda. Crie o primeiro em www.")} />}
          </div>
        </Panel>
      </div>

      <div className="quick-actions">
        <button onClick={props.openProjects}><Folder size={18} /><span>{t("Abrir gerenciador de projetos")}</span></button>
        <button onClick={props.openWww}><Folder size={18} /><span>{t("Abrir pasta www")}</span></button>
        <button onClick={props.openLogs}><Terminal size={18} /><span>{t("Ver logs e diagnóstico")}</span></button>
        <button><Shield size={18} /><span>{t("CA local e certificados")}</span></button>
      </div>
    </section>
  );
}

function ProjectsView(props: {
  t: Translate;
  projects: ProjectInfo[];
  onCreate: () => void;
  onOpenWww: () => void;
  onOpenUrl: (url: string) => void;
  onOpenProject: (domain: string) => void;
  onOpenPath: (path: string) => void;
  onOpenVhost: (domain: string) => void;
  onOpenVhostsFolder: () => void;
  onSsl: (domain: string) => void;
  onHost: (domain: string) => void;
}) {
  const { t } = props;
  return (
    <section className="view">
      <div className="view-head">
        <div><h1>{t("Projetos")}</h1><p>{t("Pastas em www com domínio local .test.")}</p></div>
        <div className="head-actions">
          <button className="btn" onClick={props.onOpenWww}><Folder size={14} /> {t("Abrir www")}</button>
          <button className="btn" onClick={props.onOpenVhostsFolder}><FileText size={14} /> VirtualHosts</button>
          <button className="btn primary" onClick={props.onCreate}><Plus size={14} /> {t("Novo projeto")}</button>
        </div>
      </div>
      <div className="project-grid">
        {props.projects.map((project) => (
          <article className="project-card" key={project.name}>
            <header>
              <span>{project.framework}</span>
              <strong>{project.name}</strong>
            </header>
            <button className="url" onClick={() => props.onOpenProject(project.domain)}>
              <Globe size={13} /> {project.domain}
            </button>
            <div className="chips">
              <span className={project.ssl_enabled ? "chip ok" : "chip"}>SSL</span>
              {project.php_version && <span className="chip ok">PHP {project.php_version}</span>}
              {project.php_cgi_port && <span className="chip">CGI :{project.php_cgi_port}</span>}
              <span className={project.host_configured ? "chip ok" : "chip warn"}>hosts</span>
              <span className="chip">{project.modified_at}</span>
            </div>
            <footer>
              <button onClick={() => props.onOpenProject(project.domain)}><ExternalLink size={13} /> {t("Abrir")}</button>
              <button onClick={() => props.onOpenPath(project.path)}><Folder size={13} /> {t("Pasta")}</button>
              <button onClick={() => props.onOpenVhost(project.domain)}><FileText size={13} /> VHost</button>
              {!project.ssl_enabled && <button onClick={() => props.onSsl(project.domain)}><Shield size={13} /> SSL</button>}
              {!project.host_configured && <button onClick={() => props.onHost(project.domain)}>hosts</button>}
            </footer>
          </article>
        ))}
        <button className="project-card add-card" onClick={props.onCreate}>
          <Plus size={22} />
          <strong>{t("Criar projeto")}</strong>
          <span>{t("pasta, domínio .test e SSL local")}</span>
        </button>
      </div>
      <Panel title="VirtualHosts">
        <div className="vhost-list">
          {props.projects.map((project) => (
            <button className="vhost-row" key={project.domain} onClick={() => props.onOpenVhost(project.domain)}>
              <FileText size={14} />
              <strong>{project.domain}.conf</strong>
              <span>{project.vhost_path}</span>
            </button>
          ))}
          {!props.projects.length && <EmptyLine text={t("Nenhum VirtualHost gerado ainda.")} />}
        </div>
      </Panel>
    </section>
  );
}

function ServicesView(props: {
  t: Translate;
  env: EnvironmentInfo | null;
  services: ServiceInfo[];
  serviceAction: (service: string, command: "start_service" | "stop_service" | "restart_service") => void;
  enableAction: (service: string, command: "enable_service" | "disable_service") => void;
  busy: string | null;
  onAllowFirewall: () => void;
  onUpdatePorts: (httpPort: number, httpsPort: number, mysqlPort: number) => void;
  onUpdateHttps: (enabled: boolean) => void;
}) {
  const { t } = props;
  const [httpPort, setHttpPort] = useState(props.env?.http_port ?? 80);
  const [httpsPort, setHttpsPort] = useState(props.env?.https_port ?? 443);
  const [mysqlPort, setMysqlPort] = useState(props.env?.mysql_port ?? 3306);
  const [httpsEnabled, setHttpsEnabled] = useState(props.env?.https_enabled ?? true);

  useEffect(() => {
    if (!props.env) return;
    setHttpPort(props.env.http_port);
    setHttpsPort(props.env.https_port);
    setMysqlPort(props.env.mysql_port);
    setHttpsEnabled(props.env.https_enabled);
  }, [props.env]);

  return (
    <section className="view">
      <div className="view-head">
        <div><h1>{t("Serviços")}</h1><p>{t("Processos locais controlados pelo Ipeenv.")}</p></div>
        <button className="btn" onClick={props.onAllowFirewall}><Shield size={14} /> {t("Liberar firewall")}</button>
      </div>
      <div className="service-grid">
        {props.services.map((service) => (
          <article className={`service-card ${service.status}`} key={service.id}>
            <header>
              <div className="service-icon">{serviceIcon[service.id] ?? <Server size={17} />}</div>
              <div>
                <h2>{service.name}</h2>
                <span>{service.version}</span>
              </div>
              <span className={`dot ${service.status === "running" ? "ok" : service.available ? "off" : "warn"}`} />
            </header>
            <dl>
              {service.id === "apache" && (
                <>
                  <div>
                    <dt>Porta HTTP</dt>
                    <dd><input type="number" min="1" max="65535" value={httpPort} onChange={(e) => setHttpPort(Number(e.target.value))} onBlur={() => props.onUpdatePorts(httpPort, httpsPort, mysqlPort)} style={{width: '100%', boxSizing: 'border-box'}} /></dd>
                  </div>
                  <div>
                    <dt>Porta HTTPS</dt>
                    <dd><input type="number" min="1" max="65535" disabled={!httpsEnabled} value={httpsPort} onChange={(e) => setHttpsPort(Number(e.target.value))} onBlur={() => props.onUpdatePorts(httpPort, httpsPort, mysqlPort)} style={{width: '100%', boxSizing: 'border-box'}} /></dd>
                  </div>
                  <div>
                    <dt>HTTPS</dt>
                    <dd>
                      <label className="switch">
                        <input type="checkbox" checked={httpsEnabled} onChange={(e) => {
                          setHttpsEnabled(e.target.checked);
                          props.onUpdateHttps(e.target.checked);
                        }} />
                        <span />
                      </label>
                    </dd>
                  </div>
                </>
              )}
              {service.id === "mysql" && (
                <div>
                  <dt>Porta MySQL</dt>
                  <dd><input type="number" min="1" max="65535" value={mysqlPort} onChange={(e) => setMysqlPort(Number(e.target.value))} onBlur={() => props.onUpdatePorts(httpPort, httpsPort, mysqlPort)} style={{width: '100%', boxSizing: 'border-box'}} /></dd>
                </div>
              )}
              <div><dt>PID</dt><dd>{service.pid ?? "-"}</dd></div>
              <div><dt>{t("Binário")}</dt><dd title={service.executable}>{service.available ? t("encontrado") : t("ausente")}</dd></div>
              <div><dt>{t("Status")}</dt><dd>{serviceStatusText(service.status, t)}</dd></div>
            </dl>
            <p>{service.last_message}</p>
            <footer>
              <label className="switch with-label">
                <input
                  type="checkbox"
                  disabled={!!props.busy}
                  checked={service.enabled}
                  onChange={() => props.enableAction(service.id, service.enabled ? "disable_service" : "enable_service")}
                />
                <span />
                <em>{service.enabled ? t("Habilitado") : t("Desabilitado")}</em>
              </label>
              <button onClick={() => props.serviceAction(service.id, "restart_service")} disabled={!!props.busy || service.status !== "running"}><RefreshCw size={13} /> {t("Reiniciar")}</button>
            </footer>
          </article>
        ))}
      </div>
    </section>
  );
}

function ToolsView(props: {
  t: Translate;
  packages: PackageEntry[];
  tools: ToolInfo[];
  progress: InstallProgress | null;
  busy: string | null;
  onOpenPackages: () => void;
  onInstallPackage: (entry: PackageEntry) => void;
  onInstallTool: (tool: string) => void;
  onLaunchTool: (tool: string) => void;
}) {
  const { t } = props;
  const grouped = props.packages.reduce<Record<string, PackageEntry[]>>((acc, entry) => {
    acc[entry.category] ??= [];
    acc[entry.category].push(entry);
    return acc;
  }, {});
  const categories = Object.keys(grouped);

  return (
    <section className="view">
      <div className="view-head">
        <div>
          <h1>{t("Ferramentas")}</h1>
          <p>{t("Catálogo do Ipeenv, terminal Cmder e utilitários locais.")}</p>
        </div>
        <button className="btn primary" onClick={props.onOpenPackages}>
          <FileText size={14} /> {t("Catálogo de pacotes")}
        </button>
      </div>

      {props.progress && (
        <InstallProgressBar progress={props.progress} t={t} />
      )}

      <div className="grid two">
        <Panel title={t("Utilitários locais")}>
          <div className="tool-list">
            {props.tools.map((tool) => (
              <div className="tool-row" key={tool.id}>
                <div>
                  <strong>{tool.name}</strong>
                  <span title={tool.install_path}>
                    {tool.kind} · {tool.installed ? t("pronto") : tool.available_source ? t("baixável") : t("sem fonte")} · {tool.install_path}
                  </span>
                </div>
                <div className="tool-actions">
                  <button onClick={() => props.onLaunchTool(tool.id)} disabled={!tool.installed || !!props.busy}>
                    {t("Abrir")}
                  </button>
                  <button onClick={() => props.onInstallTool(tool.id)} disabled={!tool.available_source || !!props.busy}>
                    {t("Atualizar")}
                  </button>
                </div>
              </div>
            ))}
          </div>
        </Panel>
      </div>

      <Panel title={t("Catálogo de pacotes ({{count}} pacotes)", { count: props.packages.length })}>
        <div className="packages">
          {categories.map((category) => (
            <section className="package-group" key={category}>
              <h2>{t(category)}</h2>
              <div>
                {grouped[category].map((entry) => (
                  <article className="package-row" key={`${entry.category}:${entry.name}`}>
                    <strong>{entry.preferred ? `* ${entry.name}` : entry.name}</strong>
                    <span title={`${entry.url}\n${entry.install_dir}`}>{entry.installed ? entry.install_dir : entry.url}</span>
                    <button disabled={entry.installed || !!props.busy} onClick={() => props.onInstallPackage(entry)}>
                      {entry.installed ? t("Instalado") : props.busy === `package:${entry.name}` ? t("Instalando") : t("Instalar")}
                    </button>
                  </article>
                ))}
              </div>
            </section>
          ))}
        </div>
      </Panel>
    </section>
  );
}

function InstallProgressBar({ progress, t }: { progress: InstallProgress; t: Translate }) {
  return (
    <div className={`project-progress install-progress ${progress.status}`}>
      <div>
        <strong>{progress.item}</strong>
        <span>{t(progress.step)} · {progress.percent}%</span>
      </div>
      <div className="progress-track">
        <span style={{ width: `${progress.percent}%` }} />
      </div>
    </div>
  );
}

function LogsView(props: { t: Translate; active: string; setActive: (kind: string) => void; logs: string }) {
  const { t } = props;
  return (
    <section className="view">
      <div className="view-head">
        <div><h1>{t("Logs")}</h1><p>{t("Últimas linhas dos logs do ambiente isolado.")}</p></div>
      </div>
      <div className="tabs">
        {["app", "apache", "mysql", "php"].map((kind) => (
          <button key={kind} className={props.active === kind ? "active" : ""} onClick={() => props.setActive(kind)}>
            {kind}
          </button>
        ))}
      </div>
      <pre className="logs">{props.logs}</pre>
    </section>
  );
}

function SettingsView({
  t,
  env,
  phpRuntimes,
  onOpenHosts,
  onOpenPhpIni,
  onTogglePhpExtension,
}: {
  t: Translate;
  env: EnvironmentInfo | null;
  phpRuntimes: PhpRuntimeInfo[];
  onOpenHosts: () => void;
  onOpenPhpIni: (version: string) => void;
  onTogglePhpExtension: (version: string, extension: string, enabled: boolean) => Promise<void>;
}) {
  const [selectedPhp, setSelectedPhp] = useState(phpRuntimes[0]?.version ?? "");
  const [extensions, setExtensions] = useState<PhpExtensionInfo[]>([]);
  const [loadingExtensions, setLoadingExtensions] = useState(false);

  useEffect(() => {
    if (!phpRuntimes.length) {
      setSelectedPhp("");
      setExtensions([]);
      return;
    }
    if (!phpRuntimes.some((runtime) => runtime.version === selectedPhp)) {
      setSelectedPhp(phpRuntimes[0].version);
    }
  }, [phpRuntimes, selectedPhp]);

  const loadExtensions = async (version: string) => {
    if (!version) return;
    setLoadingExtensions(true);
    try {
      setExtensions(await invoke<PhpExtensionInfo[]>("list_php_extensions", { version }));
    } catch {
      setExtensions([]);
    } finally {
      setLoadingExtensions(false);
    }
  };

  useEffect(() => {
    loadExtensions(selectedPhp);
  }, [selectedPhp]);

  const selectedRuntime = phpRuntimes.find((runtime) => runtime.version === selectedPhp);

  return (
    <section className="view">
      <div className="view-head">
        <div><h1>{t("Preferências")}</h1><p>{t("Configurações e caminhos do ambiente.")}</p></div>
        <button className="btn" onClick={onOpenHosts}><FileText size={14} /> {t("Abrir hosts")}</button>
      </div>
      <Panel title={t("PHP por versão")}>
        <div className="php-manager">
          <aside className="php-runtime-list">
            {phpRuntimes.map((runtime) => (
              <button
                key={runtime.path}
                className={runtime.version === selectedPhp ? "active" : ""}
                onClick={() => setSelectedPhp(runtime.version)}
              >
                <strong>PHP {runtime.version}</strong>
                <span title={runtime.path}>{runtime.name} · {runtime.extension_count} ext.</span>
              </button>
            ))}
            {!phpRuntimes.length && <EmptyLine text={t("Nenhuma versão de PHP instalada em bin/php.")} />}
          </aside>
          <div className="php-extension-panel">
            {selectedRuntime ? (
              <>
                <div className="php-panel-head">
                  <div>
                    <strong>PHP {selectedRuntime.version}</strong>
                    <span title={selectedRuntime.ini_path}>{selectedRuntime.ini_path}</span>
                  </div>
                  <button className="btn" onClick={() => onOpenPhpIni(selectedRuntime.version)}>
                    <FileText size={14} /> {t("Abrir php.ini")}
                  </button>
                </div>
                <div className="extension-list">
                  {loadingExtensions && <EmptyLine text={t("Carregando extensões...")} />}
                  {!loadingExtensions && extensions.map((extension) => (
                    <label key={extension.dll} className="extension-row">
                      <input
                        type="checkbox"
                        checked={extension.enabled}
                        onChange={(event) =>
                          onTogglePhpExtension(selectedRuntime.version, extension.dll, event.target.checked)
                            .then(() => loadExtensions(selectedRuntime.version))
                        }
                      />
                      <span>{extension.name}</span>
                      <small>{extension.dll}</small>
                    </label>
                  ))}
                  {!loadingExtensions && !extensions.length && <EmptyLine text={t("Nenhuma extensão encontrada na pasta ext desta versão.")} />}
                </div>
              </>
            ) : (
              <EmptyLine text={t("Instale uma versão de PHP pelo catálogo de pacotes para gerenciar extensões.")} />
            )}
          </div>
        </div>
      </Panel>
      <Panel title={t("Estrutura isolada")}>
        <div className="settings-grid">
          <Setting icon={<HardDrive size={16} />} label={t("Root do ambiente")} value={env?.root_dir ?? "-"} />
          <Setting icon={<Server size={16} />} label="Apache" value="bin/apache/bin/httpd.exe" />
          <Setting icon={<FileText size={16} />} label="PHP" value="bin/php/php.exe" />
          <Setting icon={<Database size={16} />} label="MySQL" value="bin/mysql/bin/mysqld.exe" />
          <Setting icon={<Shield size={16} />} label="SSL" value="etc/ssl/certs" />
          <Setting icon={<Globe size={16} />} label={t("Projetos")} value="www" />
        </div>
      </Panel>
    </section>
  );
}

function CreateProjectModal(props: {
  t: Translate;
  templates: SiteTemplate[];
  phpOptions: PhpOption[];
  httpsEnabled: boolean;
  progress: ProjectProgress | null;
  onClose: () => void;
  onOpenConfig: () => void;
  onCreate: (name: string, template: string, phpVersion: string, addHost: boolean, enableSsl: boolean) => Promise<void>;
}) {
  const { t } = props;
  const [name, setName] = useState("meu-site");
  const frameworks = Array.from(new Set(props.templates.map((item) => item.framework)));
  const [framework, setFramework] = useState(frameworks[0] ?? "Blank");
  const versions = props.templates.filter((item) => item.framework === framework);
  const [template, setTemplate] = useState(versions.find((item) => item.preferred)?.name ?? versions[0]?.name ?? "");
  const selectedTemplate = props.templates.find((item) => item.name === template) ?? versions[0];
  const compatiblePhp = props.phpOptions.filter((php) =>
    (!selectedTemplate?.php_min || versionAtLeast(php.version, selectedTemplate.php_min))
    && (!selectedTemplate?.php_max || versionAtMost(php.version, selectedTemplate.php_max))
  );
  const [phpVersion, setPhpVersion] = useState(props.phpOptions[0]?.version ?? "");

  useEffect(() => {
    const nextVersions = props.templates.filter((item) => item.framework === framework);
    if (!nextVersions.some((item) => item.name === template)) {
      setTemplate(nextVersions[0]?.name ?? "");
    }
  }, [framework, props.templates, template]);

  useEffect(() => {
    if (!props.phpOptions.some((php) => php.version === phpVersion)) {
      setPhpVersion(props.phpOptions[0]?.version ?? "");
    }
  }, [props.phpOptions, phpVersion]);

  const selectedPhpMeta = props.phpOptions.find((php) => php.version === phpVersion);
  const selectedPhpCompatible =
    (!selectedTemplate?.php_min || versionAtLeast(phpVersion, selectedTemplate.php_min))
    && (!selectedTemplate?.php_max || versionAtMost(phpVersion, selectedTemplate.php_max));

  return (
    <div className="modal-backdrop">
      <form
        className="modal"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          props.onCreate(name, template, phpVersion, true, props.httpsEnabled);
        }}
      >
        <header>
          <h2>{t("Criar rapidamente um site")}</h2>
          <div className="modal-head-actions">
            <button type="button" onClick={props.onOpenConfig}>{t("Modelos")}</button>
            <button type="button" onClick={props.onClose}>{t("Fechar")}</button>
          </div>
        </header>
        <div className="create-grid">
          <label>
            Framework
            <select value={framework} onChange={(event) => setFramework(event.target.value)}>
              {frameworks.map((item) => <option key={item} value={item}>{item}</option>)}
            </select>
          </label>
          <label>
            {t("Versão do framework")}
            <select value={template} onChange={(event) => setTemplate(event.target.value)}>
              {versions.map((item) => <option key={item.name} value={item.name}>{item.version}</option>)}
            </select>
          </label>
          <label>
            {t("Versão do PHP")}
            <select value={phpVersion} onChange={(event) => setPhpVersion(event.target.value)}>
              {props.phpOptions.map((php) => (
                <option key={php.version} value={php.version}>
                  PHP {php.version} {php.installed ? t("Instalado").toLowerCase() : t("será instalado")}
                </option>
              ))}
            </select>
          </label>
          <div className="compat-note">
            {(selectedTemplate?.php_min || selectedTemplate?.php_max)
              ? t("Requer {{requirement}} · selecionado: PHP {{version}}{{compatibility}}", {
                requirement: `${selectedTemplate?.php_min ? `PHP ${selectedTemplate.php_min}+` : t("PHP sem mínimo")}${selectedTemplate?.php_max ? t(" e < {{version}}", { version: selectedTemplate.php_max }) : ""}`,
                version: phpVersion || "-",
                compatibility: selectedPhpCompatible ? t(" (compatível)") : t(" (Ipeenv vai ajustar automaticamente para funcionar)"),
              })
              : t("Sem restrição de versão de PHP")}
            {selectedPhpMeta && !selectedPhpMeta.installable ? t(" · versão sem fonte de instalação no catálogo de pacotes") : ""}
          </div>
        </div>
        <label>
          {t("Nome do projeto")}
          <input value={name} onChange={(event) => setName(event.target.value)} placeholder="meu-site" />
          <small>{t("Domínio previsto: {{name}}.test", { name: name || "projeto" })}</small>
        </label>
        {props.progress && (
          <div className={`project-progress ${props.progress.status}`}>
            <div>
              <strong>{t(props.progress.step)}</strong>
              <span>{props.progress.project} · {props.progress.percent}%</span>
            </div>
            <div className="progress-track">
              <span style={{ width: `${props.progress.percent}%` }} />
            </div>
          </div>
        )}
        <footer>
          <button type="button" onClick={props.onClose}>{t("Cancelar")}</button>
          <button className="primary" type="submit"><Plus size={14} /> {t("Criar")}</button>
        </footer>
      </form>
    </div>
  );
}

function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return <section className="panel"><header>{title}</header>{children}</section>;
}

function Stat({ label, value, note, tone }: { label: string; value: React.ReactNode; note: string; tone?: "ok" | "warn" }) {
  return <div className={`stat ${tone ?? ""}`}><span>{label}</span><strong>{value}</strong><small>{note}</small></div>;
}

function EmptyLine({ text }: { text: string }) {
  return <div className="empty-line">{text}</div>;
}

function Setting({ icon, label, value }: { icon: React.ReactNode; label: string; value: string }) {
  return <div className="setting">{icon}<span>{label}</span><strong>{value}</strong></div>;
}

function versionAtLeast(version: string, min: string) {
  const parse = (value: string) => value.split(".").map((part) => Number.parseInt(part, 10) || 0);
  const current = parse(version);
  const minimum = parse(min);
  const len = Math.max(current.length, minimum.length);
  for (let index = 0; index < len; index += 1) {
    const left = current[index] ?? 0;
    const right = minimum[index] ?? 0;
    if (left > right) return true;
    if (left < right) return false;
  }
  return true;
}

function versionAtMost(version: string, max: string) {
  return versionAtLeast(max, version);
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
