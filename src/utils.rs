use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};

use crate::{
    AVG_LB_AREA, Circuit, CircuitResult, PHYS_RAM1, PHYS_RAM2, RamMapping, block_ram_area,
};

pub fn compute_total_area(
    circuits: &[Circuit],
    result: &CircuitResult,
    has_lutram: bool,
    lutram_fraction: f64,
    has_m8k: bool,
    has_m128k: bool,
    m8k_bits: i32,
    m128k_bits: i32,
    lbs_per_m8k: i32,
    lbs_per_m128k: i32,
    max_width_ram1: i32,
    max_width_ram2: i32,
) -> f64 {
    let logic_general: i32 = circuits.iter().map(|c| c.logic_blocks).sum();

    let extra_logic_blocks = (result.extra_luts + 9) / 10;
    let lutram_blocks = result.lutram_blocks;

    let lb_for_logic = logic_general + extra_logic_blocks + lutram_blocks;

    let mut nlb_arch = lb_for_logic;

    // LBs needed to provide enough M8K sites
    if has_m8k && result.m8k_blocks > 0 && lbs_per_m8k > 0 {
        let lb_for_m8k_sites = result.m8k_blocks * lbs_per_m8k;
        if nlb_arch < lb_for_m8k_sites {
            nlb_arch = lb_for_m8k_sites;
        }
    }

    if has_m128k && result.m128k_blocks > 0 && lbs_per_m128k > 0 {
        let lb_for_m128k_sites = result.m128k_blocks * lbs_per_m128k;
        if nlb_arch < lb_for_m128k_sites {
            nlb_arch = lb_for_m128k_sites;
        }
    }

    if has_lutram && lutram_fraction > 0.0 {
        let lb_for_lutram_capacity = ((lutram_blocks as f64) / lutram_fraction).ceil() as i32;
        if nlb_arch < lb_for_lutram_capacity {
            nlb_arch = lb_for_lutram_capacity;
        }
    }

    let nlb_f = nlb_arch as f64;

    // LB area
    let logic_area = nlb_f * AVG_LB_AREA;

    // Number of BRAM macros on chip, from LB spacing.
    let num_m8k_arch = if has_m8k && lbs_per_m8k > 0 {
        nlb_arch / lbs_per_m8k
    } else {
        0
    };
    let num_m128k_arch = if has_m128k && lbs_per_m128k > 0 {
        nlb_arch / lbs_per_m128k
    } else {
        0
    };

    let area_8k = block_ram_area(m8k_bits, max_width_ram1);
    let area_128k = block_ram_area(m128k_bits, max_width_ram2);

    let bram_area = (num_m8k_arch as f64) * area_8k + (num_m128k_arch as f64) * area_128k;

    logic_area + bram_area
}

pub fn write_mappings(path: &str, mappings: &[RamMapping]) -> io::Result<()> {
    let mut file = File::create(path)?;

    let mut sorted = mappings.to_vec();
    sorted.sort_by(|a, b| {
        a.circuit_id
            .cmp(&b.circuit_id)
            .then(a.logical_ram_id.cmp(&b.logical_ram_id))
    });

    for m in &sorted {
        writeln!(
            file,
            "{} {} {} LW {} LD {} ID {} S {} P {} Type {} Mode {} W {} D {}",
            m.circuit_id,
            m.logical_ram_id,
            m.extra_luts,
            m.logical_width,
            m.logical_depth,
            m.group_id,
            m.series,
            m.parallel,
            m.phys_type.type_id(),
            m.mode.as_str(),
            m.phys_width,
            m.phys_depth
        )?;
    }
    Ok(())
}

pub fn compute_geometric_area(logic_block_file: &str, mapped_file: &str) -> io::Result<f64> {
    // ----- Step 1: read logic blocks per circuit -----
    let mut logic_blocks_map: HashMap<i32, i32> = HashMap::new();
    let file = File::open(logic_block_file)?;
    let reader = BufReader::new(file);

    for (line_idx, line_res) in reader.lines().enumerate() {
        let line = line_res?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line_idx == 0 {
            // skip header
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
        logic_blocks_map.insert(circuit_id, logic_blocks);
    }

    //read mapped file and accumulate usage per circuit
    // maps as circuit id, lutram_blocks_used, m8k_blocks_used, m128k_blocks_used, extra_luts
    let mut usage: HashMap<i32, (i32, i32, i32, i32)> = HashMap::new();

    let file = File::open(mapped_file)?;
    let reader = BufReader::new(file);

    for line_res in reader.lines() {
        let line = line_res?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 21 {
            continue;
        }

        let circuit_id: i32 = match parts[0].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let extra_luts: i32 = match parts[2].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let series: i32 = match parts[10].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let parallel: i32 = match parts[12].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let phys_blocks = series * parallel;

        let phys_type_id: i32 = match parts[14].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let entry = usage
            .entry(circuit_id)
            .or_insert((0_i32, 0_i32, 0_i32, 0_i32));

        entry.3 += extra_luts; // accumulate extra LUTs

        match phys_type_id {
            1 => entry.0 += phys_blocks, // LUTRAM
            2 => entry.1 += phys_blocks, // M8K
            3 => entry.2 += phys_blocks, // M128K
            _ => {}
        }
    }

    //per-circuit area with SAME MODEL as compute_total_area -----
    let area_8k_block = block_ram_area(PHYS_RAM1.bits, PHYS_RAM1.max_width_non_tdp);
    let area_128k_block = block_ram_area(PHYS_RAM2.bits, PHYS_RAM2.max_width_non_tdp);

    let scale = 1.0e7_f64;
    let mut product = 1.0_f64;
    let mut count = 0_usize;

    for (cid, logic_blocks) in logic_blocks_map.iter() {
        let (lutram_used, m8k_used, m128k_used, extra_luts) =
            usage.get(cid).copied().unwrap_or((0, 0, 0, 0));

        let extra_logic_blocks = (extra_luts + 9) / 10;
        let mut nlb_arch = logic_blocks + extra_logic_blocks + lutram_used;

        let lb_for_m8k = 10 * m8k_used;
        let lb_for_m128k = 300 * m128k_used;
        let lb_for_lutram_capacity = 2 * lutram_used;

        if nlb_arch < lb_for_m8k {
            nlb_arch = lb_for_m8k;
        }
        if nlb_arch < lb_for_m128k {
            nlb_arch = lb_for_m128k;
        }
        if nlb_arch < lb_for_lutram_capacity {
            nlb_arch = lb_for_lutram_capacity;
        }

        let avail_8k = nlb_arch / 10;
        let avail_128k = nlb_arch / 300;

        let logic_area = (nlb_arch as f64) * AVG_LB_AREA;
        let bram_area = (avail_8k as f64) * area_8k_block + (avail_128k as f64) * area_128k_block;

        let total_area_circuit = logic_area + bram_area;

        let scaled = total_area_circuit / scale;
        product *= scaled;
        count += 1;
    }

    if count == 0 {
        return Ok(0.0);
    }

    let nth = 1.0 / (count as f64);
    let geom_scaled = product.powf(nth);
    let geom = geom_scaled * scale;

    Ok(geom)
}

pub fn write_csv(
    results_file: &str,
    circuits: &[Circuit],
    per_circuit: &HashMap<i32, (i32, i32, i32, i32)>,
    area_8k: f64,
    area_128k: f64,
) -> io::Result<()> {
    let mut writer = csv::Writer::from_path(results_file)?;
    writer.write_record(&[
        "Circuit",
        "LUTRAM_Blocks_used",
        "8K_BRAMs_Used",
        "Regular_LBs_used",
        "Required_LB_Tiles_in_Chip",
        "Total_FPGA_Area",
    ])?;

    for c in circuits {
        let (lutram_used, m8k_used, m128k_used, extra_luts) =
            per_circuit.get(&c.id).copied().unwrap_or((0, 0, 0, 0));

        let regular_lbs_used = c.logic_blocks + (extra_luts + 9) / 10;
        let required_lb_tiles = regular_lbs_used + lutram_used;
        let logic_area = required_lb_tiles as f64 * AVG_LB_AREA;
        let bram_area = (m8k_used as f64) * area_8k + (m128k_used as f64) * area_128k;
        let total_area_circuit = logic_area + bram_area;
        let total_area_cir_simplified = format!("{:.3}", total_area_circuit);
        //Printing csv data
        /*
        eprintln!(
            "Circuit {}: LUTRAM blocks used = {}, 8K BRAM used = {}, 128K BRAM used = {}, Required LB Tiles in Chip = {}, Total FPGA area = {:.2}",
            c.id, lutram_used, m8k_used, m128k_used, required_lb_tiles, total_area_circuit
        );
        */
        let results = (
            c.id,
            lutram_used,
            m8k_used,
            m128k_used,
            required_lb_tiles,
            total_area_cir_simplified,
        );

        writer.serialize(results)?;
    }
    writer.flush()?;
    Ok(())
}
