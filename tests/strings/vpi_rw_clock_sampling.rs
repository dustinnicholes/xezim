//! A clock driven over VPI from `cbReadWriteSynch` (how cocotb drives one).
//!
//! * IEEE 1800-2017 §4.4.2 / §16.5.1: a clocked concurrent assertion samples
//!   its operands in the Preponed region of the slot whose ReadWrite-synch
//!   delta raised the clock. The edge was first seen by the NEXT slot's tick,
//!   which had re-sampled after the edge's NBA region: a counter check
//!   `take |-> ge1` failed on the legal take that consumed the last token,
//!   one time step late, and missed the real violation.
//! * §21.7.2: a value written from a ReadWrite-synch routine that runs at the
//!   END of a slot (registered from another ReadWrite routine) is dumped at
//!   that slot's time, not at the next slot's.
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_vpirwclk_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn shared_lib(dir: &Path, src: &str) -> PathBuf {
    let c = dir.join("rwclk.c");
    let so = dir.join("rwclk.so");
    std::fs::write(&c, src).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let ok = Command::new("cc")
        .args(["-shared", "-fPIC", "-Wall", "-I"])
        .arg(&include)
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", c.display());
    so
}

/// Run the design with the VPI clock; returns (FAIL lines, VCD text).
fn run(tag: &str, hold: u8) -> (Vec<String>, String) {
    let dir = scratch(tag);
    let so = shared_lib(&dir, RWCLK_C);
    std::fs::write(
        dir.join("top.sv"),
        TOP_SV.replace("HOLD", &hold.to_string()),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--vpi-lib")
        .arg(&so)
        .args(["--no-cache", "--wave", "-s", "top", "top.sv"])
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "run failed:\n{text}");
    let fails = text
        .lines()
        .filter(|l| l.starts_with("FAIL"))
        .map(|l| l.trim().to_string())
        .collect();
    let vcd = std::fs::read_to_string(dir.join("top.vcd")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    (fails, vcd)
}

/// Time of the first change of `name` to `val` in a VCD.
fn first_change(vcd: &str, name: &str, val: char) -> Option<u64> {
    let code = vcd.lines().find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() >= 5 && f[0] == "$var" && f[4] == name).then(|| f[3].to_string())
    })?;
    let mut t = 0u64;
    for l in vcd.lines() {
        let l = l.trim();
        if let Some(n) = l.strip_prefix('#') {
            t = n.parse().ok()?;
        } else if l.len() > 1 && l.starts_with(val) && l[1..] == code {
            return Some(t);
        }
    }
    None
}

const RWCLK_C: &str = r#"
#include <stdio.h>
#include "vpi_user.h"
/* A clock driven the way cocotb drives one: a timer callback schedules a
 * cbReadWriteSynch whose routine writes the clock with vpiNoDelay. At t=30,
 * after raising the clock, the routine registers a SECOND ReadWrite callback
 * that writes `marker` (the deferred, end-of-slot delta). */
static vpiHandle clk, marker;
static int level = 0;
static PLI_INT32 put(vpiHandle h, int v) {
    s_vpi_value val; val.format = vpiIntVal; val.value.integer = v;
    vpi_put_value(h, &val, NULL, vpiNoDelay); return 0;
}
static PLI_INT32 mark_cb(p_cb_data d) { (void)d; return put(marker, 1); }
static PLI_INT32 rw_cb(p_cb_data d) {
    s_vpi_time now; now.type = vpiSimTime; vpi_get_time(NULL, &now);
    level ^= 1; put(clk, level);
    if (now.low == 30) {
        s_vpi_time t0 = { vpiSimTime, 0, 0, 0 };
        s_cb_data cb = { cbReadWriteSynch, mark_cb, NULL, &t0, NULL, 0, NULL };
        vpi_register_cb(&cb);
    }
    return 0;
}
static PLI_INT32 timer_cb(p_cb_data d) {
    s_vpi_time now; now.type = vpiSimTime; vpi_get_time(NULL, &now);
    s_vpi_time t0 = { vpiSimTime, 0, 0, 0 };
    s_cb_data rw = { cbReadWriteSynch, rw_cb, NULL, &t0, NULL, 0, NULL };
    vpi_register_cb(&rw);
    if (now.low < 100) {
        s_vpi_time t5 = { vpiSimTime, 0, 5, 0 };
        s_cb_data tm = { cbAfterDelay, timer_cb, NULL, &t5, NULL, 0, NULL };
        vpi_register_cb(&tm);
    }
    return 0;
}
static PLI_INT32 start_cb(p_cb_data d) {
    (void)d;
    clk = vpi_handle_by_name("top.clk", NULL);
    marker = vpi_handle_by_name("top.marker", NULL);
    put(clk, 0); put(marker, 0);
    s_vpi_time t5 = { vpiSimTime, 0, 5, 0 };
    s_cb_data tm = { cbAfterDelay, timer_cb, NULL, &t5, NULL, 0, NULL };
    vpi_register_cb(&tm);
    return 0;
}
static void reg(void) {
    s_cb_data cb = { cbStartOfSimulation, start_cb, NULL, NULL, NULL, 0, NULL };
    vpi_register_cb(&cb);
}
void (*vlog_startup_routines[])(void) = { reg, 0 };
"#;

const TOP_SV: &str = r#"
`timescale 1ns/1ns
module top;
  logic clk, marker, rst_n = 0, take = 0;
  logic [3:0] cnt;
  always_ff @(posedge clk or negedge rst_n)
    if (!rst_n) cnt <= 4'd1; else if (take) cnt <= cnt - 1;
  wire ge1 = (cnt != 0);
  a_ge1: assert property (@(posedge clk) disable iff (!rst_n) take |-> ge1)
    else $display("FAIL %0t", $time);
  initial begin
    $dumpfile("top.vcd"); $dumpvars(0, top);
    #12 rst_n = 1;
    @(posedge clk); #1 take = 1;       // takes the one token at the next edge (legal)
    @(posedge clk); #1 take = HOLD;    // HOLD=1: takes again with cnt==0 (violation)
    @(posedge clk); #1 take = 0;
    #40 $finish;
  end
endmodule
"#;

#[test]
fn assertion_samples_preponed_values_at_a_readwrite_synch_edge() {
    let (fails, _) = run("clean", 0);
    assert!(
        fails.is_empty(),
        "legal take of the last token reported: {fails:?}"
    );
    // Taking again with no token fails at that edge (t=35) and only there.
    let (fails, _) = run("bad", 1);
    assert_eq!(fails, vec!["FAIL 35"]);
}

#[test]
fn readwrite_synch_write_at_slot_end_is_dumped_at_its_time() {
    let (_, vcd) = run("vcd", 0);
    assert_eq!(
        first_change(&vcd, "marker", '1'),
        Some(30),
        "marker written at t=30"
    );
}
