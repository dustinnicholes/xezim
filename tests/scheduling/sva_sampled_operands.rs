//! IEEE 1800-2017 §16.5.1 / §16.9.3: every operand of a concurrent assertion
//! is read through its SAMPLED (Preponed) value, including
//!
//! * the operand of `$stable/$rose/$fell/$changed/$past` — a signal that the
//!   checking edge itself updates through an NBA must still compare equal;
//! * a compound operand (`$past(a && b)`, `$stable(v[2:1])`), which is the
//!   expression evaluated over its operands' sampled values;
//! * a hierarchical operand inside a NAMED property (`property p; u.x |-> …`),
//!   which reaches the evaluator as a member-access chain rather than the
//!   flat identifier an inline assertion gets.
//!
//! * a NESTED sampled-value function (`$past($past(a))`): the inner history
//!   takes this tick's sample before the outer one reads it, so it equals
//!   `$past(a, 2)`.
//!
//! §27.4 / §16.14: a concurrent assertion in a generate-for loop is one
//! assertion PER generated scope; each sees its own genvar value, in the
//! property body and in its action blocks, in every instance of the module.
//!
//! Each case pairs a clean design (no FAIL line) with a violating one whose
//! FAIL lines are exactly the edges where the sampled values violate the
//! property.
use std::path::PathBuf;
use std::process::Command;

fn run(name: &str, top: &str, src: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sva_sampled_operands");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(format!("{name}.sv"));
    std::fs::write(&sv, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", top, "--no-cache", sv.to_str().unwrap()])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

/// The FAIL lines the run printed, sorted.
fn fails(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .lines()
        .filter(|l| l.starts_with("FAIL"))
        .map(|l| l.trim().to_string())
        .collect();
    v.sort();
    v
}

/// `freeze` holds `q` for one cycle and is released on the edge that checks
/// `|=> $stable(q)`, where `q` updates through its NBA. The sampled values
/// on both edges are equal, so the property holds.
const STABLE_SRC: &str = r#"
module t;
  logic clk = 0, rst_n = 0, freeze = 0;
  logic [7:0] q;
  always #5 clk = ~clk;
  always_ff @(posedge clk or negedge rst_n)
    if (!rst_n) q <= 8'h10; else if (HOLD) q <= q + 1;
  a_stable: assert property (@(posedge clk) disable iff (!rst_n) freeze |=> $stable(q))
    else $display("FAIL stable %0t", $time);
  a_past: assert property (@(posedge clk) disable iff (!rst_n) freeze |=> q == $past(q))
    else $display("FAIL past %0t", $time);
  initial begin
    #12 rst_n = 1;
    @(posedge clk); #1 freeze = 1;
    @(posedge clk); #1 freeze = 0;
    repeat (3) @(posedge clk);
    #1 $finish;
  end
endmodule
"#;

#[test]
fn stable_operand_updated_on_the_checking_edge() {
    let clean = run("stable_clean", "t", &STABLE_SRC.replace("HOLD", "!freeze"));
    assert!(fails(&clean).is_empty(), "clean design reported:\n{clean}");
    // A design that ignores `freeze` changes `q` across the frozen cycle:
    // both forms fail at the checking edge, t=35, and nowhere else.
    let bad = run("stable_bad", "t", &STABLE_SRC.replace("HOLD", "1"));
    assert_eq!(fails(&bad), vec!["FAIL past 35", "FAIL stable 35"], "{bad}");
}

/// `a && b` is true only in the cycle before `req` rises. `$past(a && b)` at
/// the edge where `req` rises reads that cycle's sample, not the current one.
#[test]
fn past_of_an_expression_reads_the_previous_samples() {
    let src = r#"
module t;
  logic clk = 0, rst_n = 0, a = 0, b = 0, req = 0;
  always #5 clk = ~clk;
  a_expr: assert property (@(posedge clk) disable iff (!rst_n) $rose(req) |-> $past(a && b))
    else $display("FAIL expr %0t", $time);
  a_bare: assert property (@(posedge clk) disable iff (!rst_n) $rose(req) |-> $past(a) && $past(b))
    else $display("FAIL bare %0t", $time);
  initial begin
    #12 rst_n = 1;
    @(posedge clk); #1 a = A; b = 1;
    @(posedge clk); #1 a = 0; b = 0; req = 1;
    repeat (3) @(posedge clk);
    #1 $finish;
  end
endmodule
"#;
    let clean = run("past_expr_clean", "t", &src.replace("a = A", "a = 1"));
    assert!(fails(&clean).is_empty(), "clean design reported:\n{clean}");
    let bad = run("past_expr_bad", "t", &src.replace("a = A", "a = 0"));
    assert_eq!(fails(&bad), vec!["FAIL bare 35", "FAIL expr 35"], "{bad}");
}

/// `$stable` of a part-select changes when the selected bits do.
#[test]
fn stable_of_a_part_select() {
    let src = r#"
module t;
  logic clk = 0, rst_n = 0, req = 0; logic [3:0] v = 0;
  always #5 clk = ~clk;
  a_sel: assert property (@(posedge clk) disable iff (!rst_n) $rose(req) |-> $stable(v[2:1]))
    else $display("FAIL sel %0t", $time);
  initial begin
    #12 rst_n = 1;
    @(posedge clk); #1 v = 4'b0000;
    @(posedge clk); #1 req = 1; v = NEWV;
    repeat (3) @(posedge clk);
    #1 $finish;
  end
endmodule
"#;
    // Only bits outside [2:1] change: stable.
    let clean = run("stable_sel_clean", "t", &src.replace("NEWV", "4'b1001"));
    assert!(fails(&clean).is_empty(), "clean design reported:\n{clean}");
    let bad = run("stable_sel_bad", "t", &src.replace("NEWV", "4'b0110"));
    assert_eq!(fails(&bad), vec!["FAIL sel 35"], "{bad}");
}

/// A hierarchical operand inside a NAMED property is sampled like the same
/// operand written inline: at the edge where `u.r` first rises through its
/// NBA, its sampled value is still 0.
#[test]
fn named_property_hierarchical_operand_is_sampled() {
    let src = r#"
module sub(input logic clk, input logic go);
  logic r;
  always_ff @(posedge clk) r <= go;
endmodule
module t;
  logic clk = 0, go = 0;
  always #5 clk = ~clk;
  sub u(.*);
  property p_named; @(posedge clk) u.r |-> $past(go); endproperty
  a_named: assert property (p_named) else $display("FAIL named %0t", $time);
  a_inline: assert property (@(posedge clk) u.r |-> $past(go)) else $display("FAIL inline %0t", $time);
  initial begin STIM #30 $finish; end
endmodule
"#;
    let clean = run(
        "named_hier_clean",
        "t",
        &src.replace("STIM", "#12 go = 1; #10 go = 0;"),
    );
    assert!(fails(&clean).is_empty(), "clean design reported:\n{clean}");
    // `r` forced high from t=0 while `go` never rises: both forms fail on
    // every edge (t=5, 15, 25, 35; sorted as text).
    let bad = run(
        "named_hier_bad",
        "t",
        &src.replace("STIM", "force u.r = 1; #12;"),
    );
    assert_eq!(
        fails(&bad),
        vec![
            "FAIL inline 15",
            "FAIL inline 25",
            "FAIL inline 35",
            "FAIL inline 5",
            "FAIL named 15",
            "FAIL named 25",
            "FAIL named 35",
            "FAIL named 5"
        ],
        "{bad}"
    );
}

/// One assertion per generate-for iteration, each with its own genvar (in
/// the body and the action block), in every instance.
#[test]
fn generate_for_assertion_per_iteration_and_instance() {
    let src = r#"
module sub #(parameter logic [2:0] E = 3'b010) (input logic clk, input logic v);
  for (genvar l = 0; l < 3; l++) begin : g
    a_elem: assert property (@(posedge clk) v |-> E[l] == 1'b0)
      else $display("FAIL %m l=%0d %0t", l, $time);
  end
endmodule
module t;
  logic clk = 0, v = 0;
  always #5 clk = ~clk;
  sub #(.E(3'b010)) ua(.clk, .v);
  sub #(.E(3'b100)) ub(.clk, .v);
  for (genvar l = 0; l < 3; l++) begin : g_top
    a_all: assert property (@(posedge clk) v |-> 1'b0) else $display("FAIL top l=%0d %0t", l, $time);
  end
  initial begin #12 v = 1; #6 $finish; end
endmodule
"#;
    let text = run("generate_assertions", "t", src);
    assert_eq!(
        fails(&text),
        vec![
            "FAIL t.ua.g[1].a_elem l=1 15",
            "FAIL t.ub.g[2].a_elem l=2 15",
            "FAIL top l=0 15",
            "FAIL top l=1 15",
            "FAIL top l=2 15",
        ],
        "{text}"
    );
}

/// `$past($past(a))` is `$past(a, 2)`, and one more level is `$past(a, 3)`.
/// The expression history of the inner `$past(a)` used to be recorded before
/// `a`'s own history took this tick's sample, so it lagged one tick and the
/// nested form missed the violation at t=45 that the direct form reports.
/// The equivalences are checked once three ticks of history exist (`warm`),
/// so the before-history value of `$past` does not enter.
#[test]
fn nested_past_matches_past_with_depth() {
    let src = r#"
module t;
  logic clk = 0, a = 0, en = 0, warm = 0;
  always #5 clk = ~clk;
  a_n2: assert property (@(posedge clk) en |-> $past($past(a))) else $display("FAIL nested %0t", $time);
  a_d2: assert property (@(posedge clk) en |-> $past(a, 2)) else $display("FAIL direct %0t", $time);
  a_eq2: assert property (@(posedge clk) warm |-> $past($past(a)) == $past(a, 2))
    else $display("FAIL eq2 %0t", $time);
  a_eq3: assert property (@(posedge clk) warm |-> $past($past($past(a))) == $past(a, 3))
    else $display("FAIL eq3 %0t", $time);
  a_rose: assert property (@(posedge clk) warm |-> $rose($past(a)) == $past($rose(a)))
    else $display("FAIL rose %0t", $time);
  initial begin
    #12 a = 1; #10 a = 0; #10 warm = 1; #4 en = 1;
    #10 en = 0; a = 1; #10 a = 0; #10 a = 1; #20 a = 0; #30 $finish;
  end
endmodule
"#;
    // The sampled `a` two ticks before t=45 (at t=25) is 0: both forms fail
    // there and only there.
    let text = run("nested_past", "t", src);
    assert_eq!(
        fails(&text),
        vec!["FAIL direct 45", "FAIL nested 45"],
        "{text}"
    );
}
