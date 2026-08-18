//! JVM memory planning for native-mode services.
//!
//! Native mode runs every Puru service as a bare `java -jar` child process. With
//! no flags a JVM sizes its own max heap at 25% of the box's physical RAM, so a
//! dozen of them on one hospital server each believe they may take a quarter of
//! the machine. They never all cash that cheque at once, which is exactly why
//! the symptom is a slow creep into swap and a watchdog low-memory alert nobody
//! can explain rather than an outright crash.
//!
//! This module turns the box's RAM into an explicit budget: reserve what the OS,
//! MySQL, RabbitMQ and Nucleus itself need, then divide what's left across the
//! services that are actually installed, weighted by how hard each one works.
//! The result is rendered as JVM flags at spawn time and shown, with its
//! arithmetic, on the Performance screen.
//!
//! The number that matters is not the heap. A Spring Boot 3 service's resident
//! set is roughly `-Xmx` plus [`NON_HEAP_TAIL_MB`] of metaspace, code cache,
//! thread stacks, GC structures and direct buffers — so `-Xmx512m` costs the box
//! about 800 MB, not 512. Every figure here budgets the tail.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config::NucleusConfig;

// ── Tuning constants ─────────────────────────────────────────────────────────

/// Below this, a Spring Boot service with Hibernate spends more time in GC than
/// in work. Nothing is ever planned smaller.
pub const HEAP_FLOOR_MB: u64 = 192;


/// Non-heap resident cost of one Spring Boot 3 JVM: metaspace (~120 MB), code
/// cache (~50 MB), thread stacks, GC structures and direct buffers. Budgeted per
/// service on top of its heap.
pub const NON_HEAP_TAIL_MB: u64 = 280;

/// Metaspace cap. Generous next to the ~120 MB a Spring Boot 3 app really uses —
/// it exists to stop a classloader leak eating the box, not to squeeze it.
const METASPACE_CAP_MB: u64 = 256;

/// At or below this heap, SerialGC has a smaller footprint than G1 — no GC
/// thread pool sized to the core count, no region bookkeeping — and none of
/// these services care about the longer pause.
const SERIAL_GC_MAX_HEAP_MB: u64 = 256;

/// Heap sizes are rounded down to a multiple of this, so the flags stay readable.
const HEAP_GRANULARITY_MB: u64 = 64;

/// How much memory a service actually wants, which sets its share of the heap
/// pool. This is deliberately about footprint rather than request rate: a
/// service can sit on the critical path of every request and still hold very
/// little (puru-auth), while one that is called rarely can hold a great deal
/// (puru-pacs, moving DICOM studies around).
///
/// The Performance screen shows measured RSS beside the plan, so a site can
/// correct a tier from what it observes rather than from what we assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Holds large working sets — images, documents, bulk result data.
    Heavy,
    /// Ordinary request/response work over the database.
    Standard,
    /// Small, stateless, or low-volume.
    Small,
}

impl Tier {
    /// The most heap this class of service can actually put to use.
    ///
    /// Without this the budget divides the whole pool by weight, which is right
    /// when a dozen services compete for a 16 GB box and wrong when two share
    /// one: a single-purpose imaging site would hand puru-pacs a 1.8 GB heap it
    /// will fill with garbage rather than work, leaving the box no free memory
    /// for anything else. Past these figures a larger heap only defers
    /// collection, and the RAM does more good as OS page cache — which is what
    /// MySQL reads through.
    fn ceiling_mb(self) -> u64 {
        match self {
            Tier::Heavy => 1024,
            Tier::Standard => 768,
            Tier::Small => 384,
        }
    }

    fn weight(self) -> u64 {
        match self {
            Tier::Heavy => 4,
            Tier::Standard => 3,
            Tier::Small => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Heavy => "heavy",
            Tier::Standard => "standard",
            Tier::Small => "small",
        }
    }
}

const SERVICE_TIERS: &[(&str, Tier)] = &[
    // Confirmed by the people who run these: pacs and has are the heavy ones,
    // auth is small despite being on every request path.
    ("puru-pacs", Tier::Heavy),
    ("puru-has", Tier::Heavy),
    ("puru-xenon", Tier::Standard),
    ("puru-neon", Tier::Standard),
    ("puru-argon", Tier::Standard),
    ("puru-comm", Tier::Standard),
    ("puru-realtime", Tier::Standard),
    ("puru-auth", Tier::Small),
    ("puru-integration", Tier::Small),
    ("puru-mercury", Tier::Small),
    ("puru-counter", Tier::Small),
    ("puru-bridge", Tier::Small),
];

/// puru-pacs streams DICOM through direct byte buffers. `MaxDirectMemorySize`
/// silently defaults to whatever `-Xmx` is, so capping the heap would also cap
/// image transfers — this floor is applied to pacs independently of its heap.
const PACS_DIRECT_MEMORY_MB: u64 = 384;

pub fn tier_for(service: &str) -> Option<Tier> {
    SERVICE_TIERS
        .iter()
        .find(|(s, _)| *s == service)
        .map(|(_, t)| *t)
}

/// Every service this module plans for, in tier order.
pub fn planned_services() -> Vec<&'static str> {
    SERVICE_TIERS.iter().map(|(s, _)| *s).collect()
}

// ── Configuration ────────────────────────────────────────────────────────────

/// Garbage collector choice for a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GcKind {
    /// Let the JVM's own ergonomics pick.
    Default,
    Serial,
    G1,
}

impl GcKind {
    fn flag(self) -> Option<&'static str> {
        match self {
            GcKind::Default => None,
            GcKind::Serial => Some("-XX:+UseSerialGC"),
            GcKind::G1 => Some("-XX:+UseG1GC"),
        }
    }
}

/// What one service is allowed to use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ServiceMemory {
    /// Initial heap (`-Xms`).
    pub min_mb: u64,
    /// Maximum heap (`-Xmx`) — the figure everything else is derived from.
    pub max_mb: u64,
    /// `-XX:MaxMetaspaceSize`.
    pub metaspace_mb: u64,
    /// `-XX:MaxDirectMemorySize`. `None` leaves the JVM default, which is equal
    /// to the max heap.
    pub direct_mb: Option<u64>,
    pub gc: GcKind,
    /// Free-form JVM flags appended verbatim, for one-off site tuning.
    pub extra_flags: Vec<String>,
}

impl Default for ServiceMemory {
    fn default() -> Self {
        Self {
            min_mb: HEAP_FLOOR_MB / 2,
            max_mb: HEAP_FLOOR_MB,
            metaspace_mb: METASPACE_CAP_MB,
            direct_mb: None,
            gc: GcKind::Default,
            extra_flags: Vec::new(),
        }
    }
}

/// RAM held back from the JVM budget for everything else on the box.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReserveConfig {
    pub os_mb: u64,
    pub nucleus_mb: u64,
    /// MySQL: InnoDB buffer pool plus server overhead.
    pub mysql_mb: u64,
    /// RabbitMQ. Note its own `vm_memory_high_watermark` defaults to 40% of the
    /// box, far above this — pin the watermark to match or the broker will
    /// happily grow past what we reserved here.
    pub rabbitmq_mb: u64,
    /// nginx, plus headroom for the backup path (mysqldump → ZIP spikes).
    pub other_mb: u64,
}

impl Default for ReserveConfig {
    fn default() -> Self {
        // A placeholder only — real reserves come from `default_reserves`, which
        // scales them to the box. Serde needs something to fill a partial table.
        Self {
            os_mb: 2048,
            nucleus_mb: 400,
            mysql_mb: 2048,
            rabbitmq_mb: 512,
            other_mb: 600,
        }
    }
}

/// The `[performance]` section of nucleus.toml.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PerformanceConfig {
    /// Apply JVM flags at all. Off restores the old bare `java -jar` behaviour.
    pub enabled: bool,
    /// Recompute the whole plan from the box's RAM on every service start.
    /// Editing any value on the Performance screen materialises the computed
    /// plan into `services` and turns this off, so nothing stays implicit.
    pub auto_tune: bool,
    /// Kill a service that exhausts its heap instead of letting it thrash. Off
    /// by default: it changes failure semantics, and the watchdog's restart is
    /// only an improvement where the OOM is a spike rather than a leak.
    pub exit_on_oom: bool,
    /// Empty means "use the auto-computed reserves for this box".
    pub reserves: Option<ReserveConfig>,
    /// Per-service overrides. Consulted only when `auto_tune` is off; a service
    /// missing here still falls back to its computed allocation.
    pub services: HashMap<String, ServiceMemory>,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_tune: true,
            exit_on_oom: false,
            reserves: None,
            services: HashMap::new(),
        }
    }
}

// ── Planning ─────────────────────────────────────────────────────────────────

/// One service's row in the plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePlan {
    pub service: String,
    pub tier: String,
    /// False when the JAR isn't on disk — the service is shown but takes no
    /// share of the budget.
    pub installed: bool,
    pub memory: ServiceMemory,
    /// Heap + non-heap tail: what this service is expected to cost the box.
    pub estimated_rss_mb: u64,
    /// Measured RSS, when the process is running. Filled in by the caller.
    pub measured_rss_mb: Option<u64>,
    /// The exact flags that will be passed to the JVM.
    pub jvm_args: Vec<String>,
}

/// The whole budget, with its arithmetic, for the Performance screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPlan {
    pub total_ram_mb: u64,
    pub reserves: ReserveConfig,
    pub reserved_total_mb: u64,
    /// What's left for JVMs after reserves.
    pub jvm_budget_mb: u64,
    /// Sum of every installed service's estimated RSS.
    pub allocated_mb: u64,
    /// Budget minus allocation. Negative means the plan doesn't fit.
    pub headroom_mb: i64,
    /// False when the box cannot hold the installed services even at the heap
    /// floor. The shortfall is `-headroom_mb`.
    pub fits: bool,
    pub auto_tune: bool,
    pub enabled: bool,
    pub exit_on_oom: bool,
    pub services: Vec<ServicePlan>,
    /// Operator-facing notes: over-subscription, RabbitMQ's watermark, and so on.
    pub warnings: Vec<String>,
}

/// Physical RAM in MB.
pub fn total_ram_mb() -> u64 {
    use sysinfo::{System, SystemExt};
    let mut sys = System::new();
    sys.refresh_memory();
    // sysinfo 0.29 reports bytes.
    sys.total_memory() / 1024 / 1024
}

/// Reserves scaled to the box, used whenever the operator hasn't pinned their own.
pub fn default_reserves(total_mb: u64) -> ReserveConfig {
    // Windows with an AV agent idles a good deal heavier than a headless Linux box.
    let os_mb = if cfg!(windows) { 2048 } else { 1024 };

    // InnoDB's own default buffer pool is 128 MB, which is far too *small* for a
    // hospital dataset — that shows up as disk thrash, not memory pressure. Plan
    // ~22% of the box for the pool plus server overhead, clamped so a tiny box
    // keeps something and a large one doesn't hand MySQL everything.
    let mysql_mb = (total_mb * 22 / 100).clamp(768, 8192) + 600;

    let rabbitmq_mb = (total_mb * 6 / 100).clamp(512, 2048);

    ReserveConfig {
        os_mb,
        nucleus_mb: 400,
        mysql_mb,
        rabbitmq_mb,
        other_mb: 600,
    }
}

impl ReserveConfig {
    pub fn total_mb(&self) -> u64 {
        self.os_mb + self.nucleus_mb + self.mysql_mb + self.rabbitmq_mb + self.other_mb
    }
}

fn round_down(mb: u64, granularity: u64) -> u64 {
    (mb / granularity) * granularity
}

/// Derive the rest of a service's settings from its heap ceiling.
fn memory_for(service: &str, max_mb: u64) -> ServiceMemory {
    // A low -Xms lets a quiet service stay small; the heap grows on demand up to
    // max_mb. Committing the maximum up front would defeat the whole budget on a
    // box where most services idle.
    let min_mb = (max_mb / 4).clamp(64, 256);

    let gc = if max_mb <= SERIAL_GC_MAX_HEAP_MB {
        GcKind::Serial
    } else {
        GcKind::G1
    };

    // Only pacs needs direct memory decoupled from its heap; for everything else
    // the JVM default (= max heap) is more than enough.
    let direct_mb = if service == "puru-pacs" {
        Some(PACS_DIRECT_MEMORY_MB.max(max_mb))
    } else {
        None
    };

    ServiceMemory {
        min_mb,
        max_mb,
        metaspace_mb: METASPACE_CAP_MB,
        direct_mb,
        gc,
        extra_flags: Vec::new(),
    }
}

/// Split a heap pool across services by tier weight.
///
/// Returns each service's heap ceiling. When the pool can't cover the floor for
/// everyone, every service gets the floor and the caller reports the shortfall —
/// silently shrinking below [`HEAP_FLOOR_MB`] would trade a memory alert for a
/// GC-thrash outage, which is worse.
fn distribute(installed: &[&str], heap_pool_mb: i64) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    if installed.is_empty() {
        return out;
    }

    let floor_total = HEAP_FLOOR_MB * installed.len() as u64;
    if heap_pool_mb <= floor_total as i64 {
        for svc in installed {
            out.insert((*svc).to_string(), HEAP_FLOOR_MB);
        }
        return out;
    }

    let total_weight: u64 = installed
        .iter()
        .map(|s| tier_for(s).unwrap_or(Tier::Standard).weight())
        .sum();

    for svc in installed {
        let tier = tier_for(svc).unwrap_or(Tier::Standard);
        let share = (heap_pool_mb as u64) * tier.weight() / total_weight.max(1);
        // Capped at what the service can use, not at what the box can spare —
        // any surplus is left free rather than inflating heaps that don't need it.
        let heap =
            round_down(share, HEAP_GRANULARITY_MB).clamp(HEAP_FLOOR_MB, tier.ceiling_mb());
        out.insert((*svc).to_string(), heap);
    }

    out
}

/// Build the full plan for this box.
///
/// `auto_tune` computes every allocation from the live RAM figure; otherwise the
/// stored per-service values win, with the computed one as the fallback for any
/// service the operator never touched.
pub fn plan(config: &NucleusConfig) -> MemoryPlan {
    let perf = &config.performance;
    let total_ram_mb = total_ram_mb();
    let reserves = perf
        .reserves
        .clone()
        .unwrap_or_else(|| default_reserves(total_ram_mb));
    let reserved_total_mb = reserves.total_mb();
    let jvm_budget_mb = total_ram_mb.saturating_sub(reserved_total_mb);

    let all = planned_services();
    let installed: Vec<&str> = all
        .iter()
        .copied()
        .filter(|s| crate::services::native::is_installed(config, s))
        .collect();

    // Every installed JVM pays the non-heap tail before any heap is handed out.
    let tail_total = NON_HEAP_TAIL_MB * installed.len() as u64;
    let heap_pool_mb = jvm_budget_mb as i64 - tail_total as i64;
    let computed = distribute(&installed, heap_pool_mb);

    let mut warnings = Vec::new();
    let mut services = Vec::new();
    let mut allocated_mb = 0u64;

    for svc in &all {
        let is_installed = installed.contains(svc);
        let computed_max = computed.get(*svc).copied().unwrap_or(HEAP_FLOOR_MB);
        let memory = if perf.auto_tune {
            memory_for(svc, computed_max)
        } else {
            perf.services
                .get(*svc)
                .cloned()
                .unwrap_or_else(|| memory_for(svc, computed_max))
        };

        let estimated_rss_mb = memory.max_mb + NON_HEAP_TAIL_MB;
        if is_installed {
            allocated_mb += estimated_rss_mb;
        }

        services.push(ServicePlan {
            service: (*svc).to_string(),
            tier: tier_for(svc).map(Tier::as_str).unwrap_or("standard").to_string(),
            installed: is_installed,
            jvm_args: render_args(&memory, perf),
            memory,
            estimated_rss_mb,
            measured_rss_mb: None,
        });
    }

    let headroom_mb = jvm_budget_mb as i64 - allocated_mb as i64;
    let fits = headroom_mb >= 0;

    if !fits {
        warnings.push(format!(
            "This box is about {:.1} GB short for {} services. Every service is at the {} MB heap floor \
             and the total still exceeds the budget — either run fewer services here or add RAM.",
            (-headroom_mb) as f64 / 1024.0,
            installed.len(),
            HEAP_FLOOR_MB
        ));
    }

    if !perf.enabled {
        warnings.push(
            "JVM tuning is switched off — services run with no flags, so each one sizes its own \
             max heap at 25% of this box's RAM."
                .to_string(),
        );
    }

    warnings.push(format!(
        "RabbitMQ's vm_memory_high_watermark defaults to 40% of RAM (about {:.1} GB here), well above \
         the {} MB reserved for it. Pin the watermark to match, or the broker can grow into the \
         services' budget.",
        total_ram_mb as f64 * 0.4 / 1024.0,
        reserves.rabbitmq_mb
    ));

    MemoryPlan {
        total_ram_mb,
        reserves,
        reserved_total_mb,
        jvm_budget_mb,
        allocated_mb,
        headroom_mb,
        fits,
        auto_tune: perf.auto_tune,
        enabled: perf.enabled,
        exit_on_oom: perf.exit_on_oom,
        services,
        warnings,
    }
}

// ── Flag rendering ───────────────────────────────────────────────────────────

/// JVM options for one service's settings. These go *before* `-jar` on the
/// command line; anything after it is an argument to the application, not the VM.
fn render_args(memory: &ServiceMemory, perf: &PerformanceConfig) -> Vec<String> {
    let mut args = vec![
        format!("-Xms{}m", memory.min_mb),
        format!("-Xmx{}m", memory.max_mb),
        format!("-XX:MaxMetaspaceSize={}m", memory.metaspace_mb),
    ];

    if let Some(flag) = memory.gc.flag() {
        args.push(flag.to_string());
    }

    if let Some(direct) = memory.direct_mb {
        args.push(format!("-XX:MaxDirectMemorySize={}m", direct));
    }

    if perf.exit_on_oom {
        args.push("-XX:+ExitOnOutOfMemoryError".to_string());
    }

    args.extend(memory.extra_flags.iter().cloned());
    args
}

/// The JVM options to launch `service` with, resolved from config.
///
/// Returns empty when tuning is off or the service isn't one we plan for, which
/// leaves the launch exactly as it was before this module existed.
pub fn jvm_args(service: &str, config: &NucleusConfig) -> Vec<String> {
    let perf = &config.performance;
    if !perf.enabled || tier_for(service).is_none() {
        return Vec::new();
    }

    if !perf.auto_tune {
        if let Some(memory) = perf.services.get(service) {
            return render_args(memory, perf);
        }
    }

    // Auto, or a service with no stored override: take the row out of the live plan.
    plan(config)
        .services
        .into_iter()
        .find(|s| s.service == service)
        .map(|s| s.jvm_args)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_installed() -> Vec<&'static str> {
        planned_services()
    }

    /// Reserves for a 16 GB Windows box, spelled out so the expected tiers below
    /// don't move with the host this test happens to run on (Linux reserves 1 GB
    /// less for the OS, which lifts every allocation).
    fn sixteen_gb_windows_reserves() -> ReserveConfig {
        ReserveConfig {
            os_mb: 2048,
            nucleus_mb: 400,
            mysql_mb: 4204,
            rabbitmq_mb: 983,
            other_mb: 600,
        }
    }

    #[test]
    fn sixteen_gb_box_matches_the_documented_tiers() {
        let budget = 16384 - sixteen_gb_windows_reserves().total_mb();
        let pool = budget as i64 - (NON_HEAP_TAIL_MB * 12) as i64;
        let alloc = distribute(&all_installed(), pool);

        // The tiers documented for a 16 GB box: 576 / 384 / 256 MB of heap.
        // pacs and has hold the large working sets; auth is on every request
        // path but holds little, so it sits in the small tier.
        assert_eq!(alloc["puru-pacs"], 576, "heavy tier");
        assert_eq!(alloc["puru-has"], 576, "heavy tier");
        assert_eq!(alloc["puru-xenon"], 384, "standard tier");
        assert_eq!(alloc["puru-auth"], 256, "small tier");

        // The whole plan has to fit the budget it was derived from.
        let total_rss: u64 = alloc.values().map(|m| m + NON_HEAP_TAIL_MB).sum();
        assert!(total_rss <= budget, "{} > {}", total_rss, budget);
    }

    #[test]
    fn reserves_scale_with_the_box_and_leave_room_for_services() {
        for total in [8192u64, 16384, 32768, 65536] {
            let r = default_reserves(total);
            assert!(
                r.total_mb() < total,
                "reserves ({} MB) must leave something for services on a {} MB box",
                r.total_mb(),
                total
            );
        }
        // MySQL's share grows with the box but stops before it owns everything.
        assert!(default_reserves(65536).mysql_mb <= 8192 + 600);
    }

    #[test]
    fn small_box_floors_every_service_rather_than_shrinking_below_it() {
        // 8 GB cannot hold twelve services; each one should still be planned at
        // the floor so the caller can report a shortfall instead of handing out
        // heaps too small to run on.
        let reserves = default_reserves(8192);
        let budget = 8192 - reserves.total_mb();
        let pool = budget as i64 - (NON_HEAP_TAIL_MB * 12) as i64;
        assert!(pool < 0, "expected 8 GB to be over-subscribed");

        let alloc = distribute(&all_installed(), pool);
        assert!(alloc.values().all(|m| *m == HEAP_FLOOR_MB));
    }

    #[test]
    fn a_large_box_stops_at_what_each_service_can_use() {
        let alloc = distribute(&all_installed(), 200_000);
        for (svc, mb) in &alloc {
            assert_eq!(*mb, tier_for(svc).unwrap().ceiling_mb(), "{}", svc);
        }
    }

    #[test]
    fn a_single_purpose_site_gets_a_real_share_without_swallowing_the_box() {
        // An imaging-only 8 GB site: pacs and auth are the only JARs on disk.
        // pacs should get far more than it would on a box running all twelve,
        // but not the entire pool — the rest stays free for page cache.
        let reserves = ReserveConfig {
            os_mb: 2048,
            nucleus_mb: 400,
            mysql_mb: 2402,
            rabbitmq_mb: 512,
            other_mb: 600,
        };
        let budget = 8192 - reserves.total_mb();
        let pool = budget as i64 - (NON_HEAP_TAIL_MB * 2) as i64;
        let alloc = distribute(&["puru-pacs", "puru-auth"], pool);

        assert_eq!(alloc["puru-pacs"], Tier::Heavy.ceiling_mb());
        assert_eq!(alloc["puru-auth"], Tier::Small.ceiling_mb());

        let used: u64 = alloc.values().map(|m| m + NON_HEAP_TAIL_MB).sum();
        assert!(
            (budget as i64) - (used as i64) > 200,
            "a sparse site should keep real headroom, had {} of {}",
            used,
            budget
        );
    }

    #[test]
    fn serial_gc_below_the_threshold_g1_above() {
        assert_eq!(memory_for("puru-counter", 256).gc, GcKind::Serial);
        assert_eq!(memory_for("puru-auth", 512).gc, GcKind::G1);
    }

    #[test]
    fn pacs_gets_direct_memory_decoupled_from_its_heap() {
        // Capping the heap would otherwise cap DICOM transfers with it.
        let pacs = memory_for("puru-pacs", 384);
        assert_eq!(pacs.direct_mb, Some(PACS_DIRECT_MEMORY_MB));
        assert!(memory_for("puru-comm", 384).direct_mb.is_none());
    }

    #[test]
    fn vm_options_precede_the_jar() {
        let perf = PerformanceConfig::default();
        let args = render_args(&memory_for("puru-auth", 512), &perf);
        assert_eq!(args[0], "-Xms128m");
        assert_eq!(args[1], "-Xmx512m");
        assert!(args.iter().any(|a| a == "-XX:+UseG1GC"));
        assert!(!args.iter().any(|a| a == "-XX:+ExitOnOutOfMemoryError"));
    }

    #[test]
    fn survives_a_round_trip_through_nucleus_toml() {
        // nucleus.toml is the persistence layer for all of this, and the section
        // nests a map of structs inside an optional table — worth proving rather
        // than assuming.
        let mut config = NucleusConfig::default();
        config.performance.auto_tune = false;
        config.performance.exit_on_oom = true;
        config.performance.reserves = Some(default_reserves(16384));
        config.performance.services.insert(
            "puru-auth".to_string(),
            memory_for("puru-auth", 512),
        );

        let text = toml::to_string_pretty(&config).expect("serialise");
        let back: NucleusConfig = toml::from_str(&text).expect("parse");

        assert_eq!(back.performance, config.performance);
        assert_eq!(back.performance.services["puru-auth"].max_mb, 512);
    }

    #[test]
    fn an_absent_section_defaults_to_auto_tuning() {
        // Every existing site's nucleus.toml predates this section.
        let config: NucleusConfig = toml::from_str("hospital_code = \"TEST\"").unwrap();
        assert!(config.performance.enabled);
        assert!(config.performance.auto_tune);
        assert!(config.performance.services.is_empty());
        assert!(config.performance.reserves.is_none());
    }

    #[test]
    fn disabling_tuning_yields_no_flags() {
        let mut config = NucleusConfig::default();
        config.performance.enabled = false;
        assert!(jvm_args("puru-auth", &config).is_empty());
    }

    #[test]
    fn unknown_services_are_never_given_flags() {
        let config = NucleusConfig::default();
        assert!(jvm_args("puru-hydrogen", &config).is_empty());
    }
}
