// Proxmox Prometheus Exporter

use std::time::Duration;
use std::env;

use reqwest::Client;
use serde::Deserialize;
use tokio::time::sleep; // Библеотека для asyncio 
use prometheus::{Registry, GaugeVec, Opts, Encoder};
use axum::{Router, routing::get}; 
use tracing::{info, warn, error};

// ─────────────────────────────────────────────────────────────────────────────
// СТРУКТУРЫ JSON
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PveResponse<T> { data: T }

#[derive(Deserialize)]
struct Node {
    node:   String,
    status: Option<String>,
}

#[derive(Deserialize)]
struct NodeStatus {
    cpu:    Option<f64>,
    uptime: Option<u64>,
    memory: Option<MemBlock>,
    rootfs: Option<DiskBlock>,

}

#[derive(Deserialize)]
struct MemBlock { total: u64, used: u64 }

#[derive(Deserialize)]
struct DiskBlock { total: u64, used: u64 }

#[derive(Deserialize)]
struct VmInfo {
    vmid:   u32,
    name:   Option<String>,
    status: Option<String>,
    cpu:    Option<f64>,
    mem:    Option<u64>,
    maxmem: Option<u64>,
    uptime: Option<u64>,
}

// ─────────────────────────────────────────────────────────────────────────────
// КОНФИГУРАЦИЯ ОДНОГО КЛАСТЕРА PROXMOX
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct ClusterConfig {
    /// Метка кластера — попадёт в лейбл "cluster" в Prometheus.
    label:    String,
    /// https://host:port
    base_url: String,
    /// Заголовок авторизации: PVEAPIToken=user@realm!tokenid=secret
    auth:     String,
}

impl ClusterConfig {
    /// Парсим строку "label|host:port|tokenid|secret"
    fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.splitn(4, '|').collect();
        if parts.len() != 4 {
            error!("Неверный формат кластера: '{s}'. Ожидается label|host:port|user@realm!tokenid|secret");
            return None;
        }
        Some(ClusterConfig {
            label:    parts[0].trim().to_string(),
            base_url: format!("https://{}/api2/json", parts[1].trim()),
            // Proxmox API token auth header format
            auth:     format!("PVEAPIToken={}={}", parts[2].trim(), parts[3].trim()),
        })
    }
}

/// Читаем список кластеров из env.
/// Переменная PROXMOX_CLUSTERS содержит кластеры через запятую.
fn load_clusters() -> Vec<ClusterConfig> {
    let raw = env::var("PROXMOX_CLUSTERS")
        .expect("Задайте переменную PROXMOX_CLUSTERS");

    // split(',') — делим строку по запятой, как str.split(',') в Python
    // filter_map — map + фильтрация None значений
    let clusters: Vec<ClusterConfig> = raw
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|s| ClusterConfig::parse(s.trim()))
        .collect();

    if clusters.is_empty() {
        panic!("Не удалось разобрать ни одного кластера из PROXMOX_CLUSTERS");
    }

    info!("Загружено кластеров: {}", clusters.len());
    for c in &clusters {
        info!("  • {} → {}", c.label, c.base_url);
    }

    clusters
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP КЛИЕНТ С ТОКЕНОМ
// ─────────────────────────────────────────────────────────────────────────────

struct ProxmoxClient {
    http: Client,
}

impl ProxmoxClient {
    fn new() -> Self {
        ProxmoxClient {
            http: Client::builder()
                .danger_accept_invalid_certs(true)  // Proxmox = self-signed cert
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }

    /// GET запрос к конкретному кластеру.
    /// cluster содержит base_url и auth — клиент не привязан к одному кластеру.
    async fn get<T>(&self, cluster: &ClusterConfig, path: &str)
        -> Result<T, anyhow::Error>
    where
        T: for<'de> Deserialize<'de>,
    {
        let url = format!("{}{}", cluster.base_url, path);

        let resp = self.http
            .get(&url)
            // Токен передаётся в заголовке Authorization
            .header("Authorization", &cluster.auth)
            .send()
            .await?
            .error_for_status()?;

        let parsed: PveResponse<T> = resp.json().await?;
        Ok(parsed.data)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// МЕТРИКИ
// ─────────────────────────────────────────────────────────────────────────────

struct Metrics {
    registry: Registry,

    node_up:       GaugeVec,  // лейблы: cluster, node
    node_cpu:      GaugeVec,
    node_mem_pct:  GaugeVec,
    node_disk_pct: GaugeVec,
    node_uptime:   GaugeVec,

    vm_up:      GaugeVec,     // лейблы: cluster, node, vmid, name, type
    vm_cpu:     GaugeVec,
    vm_mem_pct: GaugeVec,
    vm_uptime:  GaugeVec,
}

impl Metrics {
    fn new() -> Self {
        let registry = Registry::new();

        let make = |registry: &Registry, name: &str, help: &str, labels: &[&str]| {
            let g = GaugeVec::new(Opts::new(name, help), labels).unwrap();
            registry.register(Box::new(g.clone())).unwrap();
            g
        };

        // Добавлен лейбл "cluster"
        let n = &["cluster", "node"];
        let v = &["cluster", "node", "vmid", "name", "type"];

        Metrics {
            node_up:       make(&registry, "proxmox_node_up",             "Узел онлайн",         n),
            node_cpu:      make(&registry, "proxmox_node_cpu_usage",      "CPU нагрузка 0–1",    n),
            node_mem_pct:  make(&registry, "proxmox_node_memory_pct",         "RAM использовано, %", n),
            node_disk_pct: make(&registry, "proxmox_node_disk_pct",           "Диск использовано, %",n),
            node_uptime:   make(&registry, "proxmox_node_uptime_seconds", "Аптайм, сек",         n),

            vm_up:      make(&registry, "proxmox_vm_up",             "VM запущена",            v),
            vm_cpu:     make(&registry, "proxmox_vm_cpu_usage",      "CPU нагрузка VM 0–1",    v),
            vm_mem_pct: make(&registry, "proxmox_vm_memory_pct",     "RAM VM использовано, %", v),
            vm_uptime:  make(&registry, "proxmox_vm_uptime_seconds", "Аптайм VM, сек",         v),

            registry,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// СБОР МЕТРИК
// ─────────────────────────────────────────────────────────────────────────────
/// Одна итерация для одного кластера.
async fn collect_cluster(client: &ProxmoxClient, cluster: &ClusterConfig, metrics: &Metrics) {
    let cname = &cluster.label; // имя кластера для лейблов

    let nodes: Vec<Node> = match client.get(cluster, "/nodes").await {
        Ok(n)  => n,
        Err(e) => { error!("[{cname}] Ошибка получения узлов: {e}"); return; }
    };

    for node in &nodes {
        let nname = &node.node;
        let online = node.status.as_deref() == Some("online");

        metrics.node_up
            .with_label_values(&[cname, nname])
            .set(if online { 1.0 } else { 0.0 });

        if !online {
            warn!("[{cname}] Узел {nname} оффлайн");
            continue;
        }

        // Статус узла
        match client.get::<NodeStatus>(cluster, &format!("/nodes/{nname}/status")).await {
            Ok(s) => {
                metrics.node_cpu.with_label_values(&[cname, nname])
                    .set(s.cpu.unwrap_or(0.0));
                metrics.node_uptime.with_label_values(&[cname, nname])
                    .set(s.uptime.unwrap_or(0) as f64);

                if let Some(m) = s.memory {
                    let pct = pct(m.used, m.total);
                    metrics.node_mem_pct.with_label_values(&[cname, nname]).set(pct);
                }
                if let Some(d) = s.rootfs {
                    let pct = pct(d.used, d.total);
                    metrics.node_disk_pct.with_label_values(&[cname, nname]).set(pct);
                }
            }
            Err(e) => warn!("[{cname}] Статус узла {nname}: {e}"),
        }

        // QEMU VM
        match client.get::<Vec<VmInfo>>(cluster, &format!("/nodes/{nname}/qemu")).await {
            Ok(vms) => record_vms(metrics, cname, nname, &vms, "qemu"),
            Err(e)  => warn!("[{cname}] VM на {nname}: {e}"),
        }

        // LXC
        match client.get::<Vec<VmInfo>>(cluster, &format!("/nodes/{nname}/lxc")).await {
            Ok(cts) => record_vms(metrics, cname, nname, &cts, "lxc"),
            Err(e)  => warn!("[{cname}] LXC на {nname}: {e}"),
        }
    }

    info!("[{cname}] OK ({} узлов)", nodes.len());
}

/// Одна итерация по всем кластерам. - на случай если несколько кластеров
async fn collect_all(client: &ProxmoxClient, clusters: &[ClusterConfig], metrics: &Metrics) {
    //// Сбрасываем ВСЕ VM метрики перед сбором
    metrics.vm_up.reset();
    metrics.vm_cpu.reset();
    metrics.vm_mem_pct.reset();
    metrics.vm_uptime.reset();

    for cluster in clusters {
        collect_cluster(client, cluster, metrics).await;
    }
}

fn record_vms(metrics: &Metrics, cluster: &str, node: &str, vms: &[VmInfo], vm_type: &str) {
    for vm in vms {
        let vmid  = vm.vmid.to_string();
        let vname = vm.name.as_deref().unwrap_or(&vmid);
        let lbl   = &[cluster, node, &vmid, vname, vm_type];

        let running = vm.status.as_deref() == Some("running");
        metrics.vm_up.with_label_values(lbl)
            .set(if running { 1.0 } else { 0.0 });

        if !running { continue; }

        metrics.vm_cpu.with_label_values(lbl).set(vm.cpu.unwrap_or(0.0));
        metrics.vm_uptime.with_label_values(lbl).set(vm.uptime.unwrap_or(0) as f64);

        if let (Some(used), Some(total)) = (vm.mem, vm.maxmem) {
            metrics.vm_mem_pct.with_label_values(lbl).set(pct(used, total));
        }
    }
}

/// Считаем процент, защищаясь от деления на ноль.
fn pct(used: u64, total: u64) -> f64 {
    if total == 0 { 0.0 } else { used as f64 / total as f64 * 100.0 }
}

// ─────────────────────────────────────────────────────────────────────────────
// MAIN
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main] // запуск функции как асинхронной точки входа
async fn main() {
    dotenvy::dotenv().ok(); // загружаем .env файл если есть

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into())
        )
        .init();

    let clusters  = load_clusters();
    let exp_port: u16 = env::var("EXPORTER_PORT").unwrap_or_else(|_| "9221".into())
                            .parse().unwrap_or(9221);
    let interval: u64 = env::var("SCRAPE_INTERVAL").unwrap_or_else(|_| "30".into())
                            .parse().unwrap_or(30);

    let client  = std::sync::Arc::new(ProxmoxClient::new());
    let metrics = std::sync::Arc::new(Metrics::new());

    // Фоновый режим (аналог while true)
    let c = client.clone();
    let m = metrics.clone();
    let cls = clusters.clone();
    tokio::spawn(async move {
        loop {
            collect_all(&c, &cls, &m).await;
            sleep(Duration::from_secs(interval)).await;
        }
    });

    // HTTP сервер
    let m2 = metrics.clone();
    let app = Router::new()
        .route("/metrics", get(move || {
            let m = m2.clone();
            async move {
                let encoder = prometheus::TextEncoder::new();
                let mut buf = Vec::new();
                encoder.encode(&m.registry.gather(), &mut buf).unwrap();
                String::from_utf8(buf).unwrap()
            }
        }))
        .route("/health", get(|| async { "OK" }));

    let addr = format!("0.0.0.0:{exp_port}");
    info!("http://{addr}/metrics");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
