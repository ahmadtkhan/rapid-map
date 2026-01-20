#![allow(dead_code)]
use std::collections::HashMap;
use std::f64;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::time::Instant;
mod utils;

pub const AVG_LB_AREA: f64 = (35000.0 + 40000.0) / 2.0;
use crate::utils::{compute_geometric_area, compute_total_area, write_csv, write_mappings};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemMode {
    Rom,
    SinglePort,
    SimpleDualPort,
    TrueDualPort,
}

impl MemMode {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "ROM" => Some(MemMode::Rom),
            "SinglePort" => Some(MemMode::SinglePort),
            "SimpleDualPort" => Some(MemMode::SimpleDualPort),
            "TrueDualPort" => Some(MemMode::TrueDualPort),
            _ => None,
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            MemMode::Rom => "ROM",
            MemMode::SinglePort => "SinglePort",
            MemMode::SimpleDualPort => "SimpleDualPort",
            MemMode::TrueDualPort => "TrueDualPort",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysType {
    Lutram,
    Ram8K,
    Ram128K,
}
impl PhysType {
    fn type_id(&self) -> i32 {
        match self {
            PhysType::Lutram => 1,
            PhysType::Ram8K => 2,
            PhysType::Ram128K => 3,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PhysConfig {
    phys_type: PhysType,
    bits: i32,
    max_width_non_tdp: i32,
    max_width_tdp: i32,
}

// Default templates
pub const PHYS_LUTRAM: PhysConfig = PhysConfig {
    phys_type: PhysType::Lutram,
    bits: 64 * 10,
    max_width_non_tdp: 20,
    max_width_tdp: 0,
};

pub const PHYS_RAM1: PhysConfig = PhysConfig {
    phys_type: PhysType::Ram8K,
    bits: 8192,
    max_width_non_tdp: 32,
    max_width_tdp: 16,
};

pub const PHYS_RAM2: PhysConfig = PhysConfig {
    phys_type: PhysType::Ram128K,
    bits: 128 * 1024,
    max_width_non_tdp: 128,
    max_width_tdp: 64,
};

#[derive(Debug)]
pub struct Memory {
    ram_id: i32,
    mode: MemMode,
    depth: i32,
    width: i32,
}

#[derive(Debug)]
pub struct Circuit {
    id: i32,
    logic_blocks: i32,
    memories: Vec<Memory>,
}

#[derive(Clone, Debug)]
pub struct RamMapping {
    circuit_id: i32,
    logical_ram_id: i32,
    extra_luts: i32,
    logical_width: i32,
    logical_depth: i32,
    group_id: i32,
    series: i32,
    parallel: i32,
    phys_type: PhysType,
    mode: MemMode,
    phys_width: i32,
    phys_depth: i32,
    phys_blocks: i32,
}
#[derive(Clone, Debug)]
pub struct CircuitResult {
    mappings: Vec<RamMapping>,
    extra_luts: i32,
    lutram_blocks: i32,
    m8k_blocks: i32,
    m128k_blocks: i32,
}

//applying physical RAM sharing
fn apply_sharing(
    mappings: &mut Vec<RamMapping>,
    m8k_cfg: Option<&PhysConfig>,
    m128k_cfg: Option<&PhysConfig>,
    m8k_blocks: &mut i32,
    m128k_blocks: &mut i32,
) {
    if let Some(cfg) = m8k_cfg {
        share_type(mappings, cfg, m8k_blocks);
    }
    if let Some(cfg) = m128k_cfg {
        share_type(mappings, cfg, m128k_blocks);
    }
}
//function to share BRAMs
fn share_type(mappings: &mut [RamMapping], cfg: &PhysConfig, total_blocks: &mut i32) {
    let phys_bits = cfg.bits;
    let max_tdp_width = cfg.max_width_tdp;

    let mut candidates: Vec<(usize, i32)> = Vec::new();

    for (idx, m) in mappings.iter().enumerate() {
        if m.phys_type != cfg.phys_type {
            continue;
        }
        if m.mode != MemMode::Rom && m.mode != MemMode::SinglePort {
            continue;
        }
        if m.phys_blocks != 1 {
            continue;
        }
        if max_tdp_width > 0 && m.phys_width > max_tdp_width {
            continue;
        }

        let logical_bits = m.logical_width * m.logical_depth;
        if logical_bits <= 0 || logical_bits >= phys_bits {
            continue;
        }
        candidates.push((idx, logical_bits));
    }

    let mut already_shared = vec![false; mappings.len()];
    for i in 0..candidates.len() {
        let (idx_i, bits_i) = candidates[i];
        if already_shared[idx_i] {
            continue;
        }

        for i in 0..candidates.len() {
            let (idx_i, _bits_i) = candidates[i];
            if already_shared[idx_i] {
                continue;
            }

            for j in (i + 1)..candidates.len() {
                let (idx_j, bits_j) = candidates[j];
                if already_shared[idx_j] {
                    continue;
                }
                if mappings[idx_i].circuit_id != mappings[idx_j].circuit_id {
                    continue;
                }

                // NEW: only share if physical shape is identical. Avoids id mismatch when mapping
                if mappings[idx_i].phys_width != mappings[idx_j].phys_width
                    || mappings[idx_i].phys_depth != mappings[idx_j].phys_depth
                    || mappings[idx_i].series != mappings[idx_j].series
                    || mappings[idx_i].parallel != mappings[idx_j].parallel
                {
                    continue;
                }
                //checking depth so it does not exceed
                let total_phys_depth = mappings[idx_i].phys_depth * mappings[idx_i].series;

                // how much depth the two logical RAMs would collectively need
                let combined_logical_depth =
                    mappings[idx_i].logical_depth + mappings[idx_j].logical_depth;

                if combined_logical_depth > total_phys_depth {
                    continue;
                }

                if bits_i + bits_j == phys_bits {
                    already_shared[idx_i] = true;
                    already_shared[idx_j] = true;

                    let gid = mappings[idx_i].group_id;

                    mappings[idx_i].mode = MemMode::TrueDualPort;
                    mappings[idx_j].mode = MemMode::TrueDualPort;
                    mappings[idx_j].group_id = gid;

                    *total_blocks -= 1;
                    break;
                }
            }
        }
    }
}

//reading data with error-handling
fn read_data(logic_block_file: &str, logic_rams_file: &str) -> io::Result<Vec<Circuit>> {
    let file = File::open(logic_block_file)?;
    let reader = BufReader::new(file);
    let mut circuits_map: HashMap<i32, Circuit> = HashMap::new();

    for (line_idx, line_res) in reader.lines().enumerate() {
        let line = line_res?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line_idx == 0 {
            //skipping first line
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let circuit_id: i32 = match parts[0].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let logic_blocks: i32 = match parts[1].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        circuits_map.insert(
            circuit_id,
            Circuit {
                id: circuit_id,
                logic_blocks,
                memories: Vec::new(),
            },
        );
    }

    let file = File::open(logic_rams_file)?;
    let reader = BufReader::new(file);

    for (line_idx, line_res) in reader.lines().enumerate() {
        let line = line_res?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line_idx < 2 {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }

        let circuit_id: i32 = match parts[0].parse() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("Bad circuit id: {}", parts[0]);
                continue;
            }
        };
        let ram_id: i32 = match parts[1].parse() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("Bad ram id: {}", parts[1]);
                continue;
            }
        };

        let mode_str = parts[2];
        let mode = match MemMode::from_str(mode_str) {
            Some(m) => m,
            None => {
                eprintln!("Unknown RAM mode: {}", mode_str);
                continue;
            }
        };

        let depth: i32 = match parts[3].parse() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("Bad depth: {}", parts[3]);
                continue;
            }
        };
        let width: i32 = match parts[4].parse() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("Bad width: {}", parts[4]);
                continue;
            }
        };

        let mem = Memory {
            ram_id,
            mode,
            depth,
            width,
        };

        let entry = circuits_map.entry(circuit_id).or_insert(Circuit {
            id: circuit_id,
            logic_blocks: 0,
            memories: Vec::new(),
        });
        entry.memories.push(mem);
    }

    let mut circuits: Vec<Circuit> = circuits_map.into_values().collect();
    circuits.sort_by_key(|c| c.id);
    Ok(circuits)
}

fn block_ram_area(bits: i32, max_width: i32) -> f64 {
    let bits_f = bits as f64;
    9000.0 + 5.0 * bits_f + 90.0 * bits_f.sqrt() + 600.0 * 2.0 * (max_width as f64)
}
fn decoder_luts(s: i32) -> i32 {
    if s <= 1 {
        0
    } else if s == 2 {
        1
    } else {
        s
    }
}

fn mux_luts(s: i32, width: i32) -> i32 {
    if s <= 1 {
        return 0;
    }
    let mut n = s;
    let mut total_nodes = 0;
    while n > 1 {
        let level_nodes = (n + 3) / 4;
        total_nodes += level_nodes;
        n = level_nodes;
    }
    let luts_per_bit = total_nodes;
    width * luts_per_bit
}

fn mapping_cost(mapping: &RamMapping, cfg: &PhysConfig) -> f64 {
    let lb_for_extra_luts = (mapping.extra_luts + 9) / 10;

    let base_area = match cfg.phys_type {
        PhysType::Lutram => {
            let lb_total = mapping.phys_blocks + lb_for_extra_luts;
            (lb_total as f64) * AVG_LB_AREA
        }
        PhysType::Ram8K | PhysType::Ram128K => {
            let lb_area = (lb_for_extra_luts as f64) * AVG_LB_AREA;
            let max_width = match mapping.mode {
                MemMode::TrueDualPort => cfg.max_width_tdp,
                _ => cfg.max_width_non_tdp,
            };
            let bram_area_per_block = block_ram_area(cfg.bits, max_width);
            lb_area + (mapping.phys_blocks as f64) * bram_area_per_block
        }
    };

    let logical_bits = (mapping.logical_width as i64 * mapping.logical_depth as i64) as f64;
    let phys_bits = (mapping.phys_blocks as i64 * cfg.bits as i64) as f64;
    let u = if phys_bits > 0.0 {
        (logical_bits / phys_bits).max(0.0).min(1.0)
    } else {
        1.0
    };

    let penalty_strength = match cfg.phys_type {
        PhysType::Lutram => 1.6,
        PhysType::Ram8K => 2.2,
        PhysType::Ram128K => 5.0,
    };
    let penalty_factor = 10.0 + penalty_strength * (10.0 - u);
    base_area * penalty_factor
}
fn best_mapping_for_phys_type(
    circuit_id: i32,
    mem: &Memory,
    group_id: i32,
    cfg: &PhysConfig,
) -> Option<(RamMapping, f64)> {
    if mem.mode == MemMode::TrueDualPort && cfg.max_width_tdp == 0 {
        return None;
    }
    let max_width = match mem.mode {
        MemMode::TrueDualPort => cfg.max_width_tdp,
        _ => cfg.max_width_non_tdp,
    };
    if max_width <= 0 {
        return None;
    }
    let width_candidates: Vec<i32> = match cfg.phys_type {
        PhysType::Lutram => vec![10, 20],
        _ => {
            let mut v = Vec::new();
            let mut w = 1;
            while w <= max_width {
                v.push(w);
                w *= 2;
            }
            v
        }
    };

    let mut best: Option<(RamMapping, f64)> = None;
    for &w_phys in &width_candidates {
        if w_phys > max_width {
            continue;
        }
        let d_phys = match cfg.phys_type {
            PhysType::Lutram => {
                if w_phys == 10 {
                    64
                } else if w_phys == 20 {
                    32
                } else {
                    continue;
                }
            }
            _ => {
                if cfg.bits % w_phys != 0 {
                    continue;
                }
                cfg.bits / w_phys
            }
        };
        if d_phys <= 0 {
            continue;
        }
        let mut p = mem.width / w_phys;
        if mem.width % w_phys != 0 {
            p += 1;
        }

        let mut s = mem.depth / d_phys;
        if mem.depth % d_phys != 0 {
            s += 1;
        }

        if s <= 0 || p <= 0 {
            continue;
        }
        if s > 16 {
            continue;
        }
        let mut extra_luts = decoder_luts(s) + mux_luts(s, mem.width);

        if s > 1 && mem.mode == MemMode::TrueDualPort {
            extra_luts *= 2;
        }

        let mapping = RamMapping {
            circuit_id,
            logical_ram_id: mem.ram_id,
            extra_luts,
            logical_width: mem.width,
            logical_depth: mem.depth,
            group_id,
            series: s,
            parallel: p,
            phys_type: cfg.phys_type,
            mode: mem.mode,
            phys_width: w_phys,
            phys_depth: d_phys,
            phys_blocks: s * p,
        };

        let cost = mapping_cost(&mapping, cfg);

        match &mut best {
            None => best = Some((mapping, cost)),
            Some((_, best_cost)) => {
                if cost < *best_cost {
                    best = Some((mapping, cost));
                }
            }
        }
    }

    best
}

// memory mapper
fn choose_mapping_for_memory(
    circuit_id: i32,
    mem: &Memory,
    group_id: i32,
    has_lutram: bool,
    has_m8k: bool,
    has_m128k: bool,
    m8k_cfg: &PhysConfig,
    m128k_cfg: &PhysConfig,
) -> RamMapping {
    let mut best_mapping: Option<RamMapping> = None;
    let mut best_cost = f64::INFINITY;

    if has_lutram {
        if let Some((m, cost)) = best_mapping_for_phys_type(circuit_id, mem, group_id, &PHYS_LUTRAM)
        {
            if cost < best_cost {
                best_cost = cost;
                best_mapping = Some(m);
            }
        }
    }

    if has_m8k {
        if let Some((m, cost)) = best_mapping_for_phys_type(circuit_id, mem, group_id, m8k_cfg) {
            if cost < best_cost {
                best_cost = cost;
                best_mapping = Some(m);
            }
        }
    }

    if has_m128k {
        if let Some((m, cost)) = best_mapping_for_phys_type(circuit_id, mem, group_id, m128k_cfg) {
            if cost < best_cost {
                //best_cost = cost;
                best_mapping = Some(m);
            }
        }
    }

    best_mapping.unwrap_or_else(|| {
        panic!(
            "No legal mapping for logical RAM {} in circuit {} under current memory config",
            mem.ram_id, circuit_id
        )
    })
}

fn assign_ram(
    circuits: &[Circuit],
    has_lutram: bool,
    has_m8k: bool,
    has_m128k: bool,
    m8k_bits: i32,
    m128k_bits: i32,
    max_width_ram1: i32,
    max_width_ram2: i32,
) -> CircuitResult {
    // dynamic configs for both memories
    let m8k_cfg = PhysConfig {
        phys_type: PhysType::Ram8K,
        bits: m8k_bits,
        max_width_non_tdp: max_width_ram1,
        max_width_tdp: max_width_ram1 / 2,
    };
    let m128k_cfg = PhysConfig {
        phys_type: PhysType::Ram128K,
        bits: m128k_bits,
        max_width_non_tdp: max_width_ram2,
        max_width_tdp: max_width_ram2 / 2,
    };
    let mut mappings = Vec::new();
    let mut extra_luts_total = 0;
    let mut lutram_blocks = 0;
    let mut m8k_blocks = 0;
    let mut m128k_blocks = 0;
    let mut next_group_id = 0;

    for c in circuits {
        for mem in &c.memories {
            let mapping = choose_mapping_for_memory(
                c.id,
                mem,
                next_group_id,
                has_lutram,
                has_m8k,
                has_m128k,
                &m8k_cfg,
                &m128k_cfg,
            );
            next_group_id += 1;

            extra_luts_total += mapping.extra_luts;
            match mapping.phys_type {
                PhysType::Lutram => lutram_blocks += mapping.phys_blocks,
                PhysType::Ram8K => m8k_blocks += mapping.phys_blocks,
                PhysType::Ram128K => m128k_blocks += mapping.phys_blocks,
            }

            mappings.push(mapping);
        }
    }

    // sharing uses dynamic configs
    let m8k_cfg_opt = if has_m8k { Some(&m8k_cfg) } else { None };
    let m128k_cfg_opt = if has_m128k { Some(&m128k_cfg) } else { None };
    apply_sharing(
        &mut mappings,
        m8k_cfg_opt,
        m128k_cfg_opt,
        &mut m8k_blocks,
        &mut m128k_blocks,
    );

    CircuitResult {
        mappings,
        extra_luts: extra_luts_total,
        lutram_blocks,
        m8k_blocks,
        m128k_blocks,
    }
}
fn main() -> io::Result<()> {
    let start = Instant::now();
    let results_file = "results.csv";

    let logic_block_file = "logic_block_count.txt";
    let logic_rams_file = "logical_rams.txt";

    let mut has_lutram = true;
    let mut lutram_fraction: f64 = 0.5;

    let mut has_ram1 = true;
    let mut ram1_bits: i32 = 8192;
    let mut lbs_per_ram1: i32 = 10;
    let mut max_width_ram1: i32 = 32;

    let mut has_ram2 = true;
    let mut ram2_bits: i32 = 128 * 1024;
    let mut lbs_per_ram2: i32 = 300;
    let mut max_width_ram2: i32 = 128;

    let args: Vec<String> = std::env::args().collect();
    if let Some(p_idx) = args.iter().position(|s| s == "-p") {
        let base = p_idx + 1;
        if args.len() < base + 10 {
            eprintln!(
                "Error: -p expects 10 arguments:\n\
                 \thas_lutram lutram_fraction \
                 has_ram1 ram1_bits lbs_per_ram1 max_width_ram1 \
                 has_ram2 ram2_bits lbs_per_ram2 max_width_ram2"
            );
            std::process::exit(1);
        }
        let get = |off: usize| &args[base + off];

        let parse_bool = |s: &str| -> Option<bool> {
            match s.to_ascii_lowercase().as_str() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                _ => None,
            }
        };

        if let Some(b) = parse_bool(get(0)) {
            has_lutram = b;
        }

        if let Ok(v) = get(1).parse::<f64>() {
            if (0.0..=1.0).contains(&v) {
                lutram_fraction = v;
            } else {
                eprintln!(
                    "Warning: lutram_fraction {} is not between 0 and 1, keeping default {}",
                    v, lutram_fraction
                );
            }
        }

        if let Some(b) = parse_bool(get(2)) {
            has_ram1 = b;
        }
        if let Ok(v) = get(3).parse::<i32>() {
            ram1_bits = v;
        }
        if let Ok(v) = get(4).parse::<i32>() {
            lbs_per_ram1 = v;
        }
        if let Ok(v) = get(5).parse::<i32>() {
            max_width_ram1 = v;
        }
        if let Some(b) = parse_bool(get(6)) {
            has_ram2 = b;
        }
        if let Ok(v) = get(7).parse::<i32>() {
            ram2_bits = v;
        }
        if let Ok(v) = get(8).parse::<i32>() {
            lbs_per_ram2 = v;
        }
        if let Ok(v) = get(9).parse::<i32>() {
            max_width_ram2 = v;
        }
    }

    // require atleast one memory type
    if !has_lutram && !has_ram1 && !has_ram2 {
        panic!("At least one memory type (LUTRAM, M8K, or M128K) must be enabled");
    }

    let circuits = read_data(logic_block_file, logic_rams_file)?;
    //Print circuit numbers
    //eprintln!("Read {} circuits", circuits.len());

    let result = assign_ram(
        &circuits,
        has_lutram,
        has_ram1,
        has_ram2,
        ram1_bits,
        ram2_bits,
        max_width_ram1,
        max_width_ram2,
    );

    let _global_total_area = compute_total_area(
        &circuits,
        &result,
        has_lutram,
        lutram_fraction,
        has_ram1,
        has_ram2,
        ram1_bits,
        ram2_bits,
        lbs_per_ram1,
        lbs_per_ram2,
        max_width_ram1,
        max_width_ram2,
    );

    let mut per_circuit: HashMap<i32, (i32, i32, i32, i32)> = HashMap::new();

    for c in &circuits {
        per_circuit.entry(c.id).or_insert((0, 0, 0, 0));
    }

    for m in &result.mappings {
        let entry = per_circuit.entry(m.circuit_id).or_insert((0, 0, 0, 0));
        match m.phys_type {
            PhysType::Lutram => entry.0 += m.phys_blocks,
            PhysType::Ram8K => entry.1 += m.phys_blocks,
            PhysType::Ram128K => entry.2 += m.phys_blocks,
        }
        entry.3 += m.extra_luts;
    }

    //Write components and blocks in the circuit used
    let area_8k = block_ram_area(ram1_bits, max_width_ram1);
    let area_128k = block_ram_area(ram2_bits, max_width_ram2);
    write_csv(results_file, &circuits, &per_circuit, area_8k, area_128k)?;
    let elapsed = start.elapsed();
    //Printing runtime
    eprintln!("Program runtime: {:.3?}", elapsed);
    //write out the RAM mapping file
    write_mappings("ram_mapped.txt", &result.mappings)?;

    //Compute geometric area
    let geom_area = compute_geometric_area(logic_block_file, "ram_mapped.txt")?;
    eprintln!("Geometric mean FPGA area = {:.5e}", geom_area);

    Ok(())
}
