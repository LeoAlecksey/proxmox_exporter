# Proxmox_Exporter

This my first project on Rust.

Proxmox-exporter
Этот проект создан на Rust и использует cargo для компиляции кода.
Если вы не уверены в своих знаниях, обратитесь к владельцу проекта.

## 1. Build

```rust
cargo run
cargo build --release # или релизная сборка:
```

## 2. Configure user & token in Proxmox

### 2.1 Создаём роль с минимальными привилегиями только на чтение

```bash
pveum role add MonitoringRO --privs "VM.Audit,Datastore.Audit,Sys.Audit,SDN.Audit"
```

### 2.2 Назначаем роль на корень / — распространяется на все ноды, VM, хранилища

```bash
pveum acl modify / --user monitoring@pve --role MonitoringRO
```

Далее выпускаем токен.

### 3. Start use

Перенести бинарник `/target/release/proxmox-exporter` на сервер и создать рядом с ним .env с наполнением как в env.example.
обязательно sudo chmod 700 ./proxmox-exporter
Далее запускаем как обычный бинарник
`./proxmox-exporter'.

### 4. Systemd configuration

```bash
cat > /etc/systemd/system/proxmox-exporter.service << 'EOF'
[Unit]
Description=Proxmox VE Prometheus Exporter
After=network.target

[Service]
Type=simple
WorkingDirectory=/proxmox_exporter
ExecStart=/proxmox_exporter/proxmox-exporter
Restart=on-failure
RestartSec=10
[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now proxmox-exporter
curl http://localhost:9221/metrics | grep proxmox

```

### 5. Metricks

prometheus.yml — добавьте в scrape_configs:

```yaml
scrape_configs:
  - job_name: proxmox
    static_configs:
      - targets:
          - "localhost:9221"   # или IP сервера где запущен экспортер
    scrape_interval: 30s
    scrape_timeout:  20s
```

### 6. Add new Metrics

Краткая схема — что и где менять

Новые данные из API
        │
        ▼
① struct NodeStatus    ← добавить поле
        │
        ▼
② struct Metrics       ← добавить GaugeVec
        │
        ▼
③ Metrics::new()       ← зарегистрировать через make()
        │
        ▼
④ collect_cluster()    ← записать значение через .set()

### Ссылки на json для выбора метрик

```markdown
http://<PVE_IP>:8006/api2/json| 'URL'/nodes/<node_name>/status
```

| url | description |
| --- | --- |
| 'URL' | http://<PVE_IP>:8006/api2/json |
| 'URL'/nodes | список узлов |
| 'URL'/nodes/<node_name>/status | CPU, RAM, диск, сеть, uptime |
| 'URL'/nodes/<node_name>/rrddata | история метрик |
| 'URL'/nodes/<node_name>/network | сетевые интерфейсы ноды |
| 'URL'/nodes/<node_name>/storage | хранилища |
| 'URL'/nodes/<node_name>/storage/{stor}/status | детали хранилища |
| --- | --- |
| 'URL'/nodes/<node_name>/qemu | список VM |
| 'URL'/nodes/<node_name>/qemu/{vmid}/status/current | статус VM |
| 'URL'/nodes/<node_name>/qemu/{vmid}/rrddata | история VM |
| 'URL'/nodes/<node_name>/qemu/{vmid}/config | конфиг VM (сколько RAM/CPU выделено) |
| --- | --- |
| 'URL'/nodes/<node_name>/lxc | список контейнеров |
| 'URL'/nodes/<node_name>/lxc/{vmid}/status/current | статус LXC |
| 'URL'/nodes/<node_name>/lxc/{vmid}/rrddata | история LXC |
| --- | --- |
| 'URL'/cluster/resources | ВСЁ сразу (узлы + VM + хранилища) |
| 'URL'/cluster/status | статус кластера (quorum и т.д.) |

## Links

- [documentation](https://doc.rust-lang.ru/book/ch01-03-hello-cargo.html)

## Support

LeoALecksey

## Links

documentation
