//! §9.2.2.4 / §9.4.2: an edge-triggered block executes on EVERY edge of its
//! event. The event-driven edge skip may elide an execution only when none
//! of the block's inputs changed, so its read set must include a signal read
//! through a DYNAMIC part-select (`m[p]` in a loop, `v[i*W +: W]`). That
//! read matched no arm of the read-set builder, so a block whose only
//! changing input was read that way was skipped and its flop held a stale
//! value until some other input moved.
use xezim::simulate;

fn bits(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("t.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", name))
}

const SRC: &str = r#"
module t;
  logic clk = 0, rst_n = 0;
  logic [5:0][10:0] m;
  logic [5:0][11:0] r_nba, r_blk, r_rst;
  logic [11:0] r_scalar;
  logic [11:0] s1_nba, s1_blk, s1_rst, s1_scalar;
  logic [11:0] s2_nba, s2_blk, s2_rst, s2_scalar;
  always @(posedge clk) for (int p = 0; p < 6; p++) r_nba[p] <= 12'(m[p]);
  always @(posedge clk) begin for (int p = 0; p < 6; p++) r_blk[p] = 12'(m[p]); end
  always_ff @(posedge clk or negedge rst_n)
    if (!rst_n) r_rst <= '0;
    else for (int p = 0; p < 6; p++) r_rst[p] <= 12'(m[p]);
  always @(posedge clk) r_scalar <= 12'(m[0]);
  always #5 clk = ~clk;
  initial begin
    m = '0; m[0] = 3;
    #2 rst_n = 1;
    repeat (2) @(posedge clk);
    #1 {s1_nba, s1_blk, s1_rst, s1_scalar} = {r_nba[0], r_blk[0], r_rst[0], r_scalar};
    m[0] = 2;                 // the only input that changes
    @(posedge clk);
    #1 {s2_nba, s2_blk, s2_rst, s2_scalar} = {r_nba[0], r_blk[0], r_rst[0], r_scalar};
    $finish;
  end
endmodule
"#;

#[test]
fn dynamic_part_select_is_an_edge_block_input() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    for n in ["s1_nba", "s1_blk", "s1_rst", "s1_scalar"] {
        assert_eq!(bits(&sim, n), 3, "{n}");
    }
    for n in ["s2_nba", "s2_blk", "s2_rst", "s2_scalar"] {
        assert_eq!(bits(&sim, n), 2, "{n}: block skipped although m changed");
    }
}
