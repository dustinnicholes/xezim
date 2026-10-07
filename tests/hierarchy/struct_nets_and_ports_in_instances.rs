//! Packed-struct NETS and struct-typed output PORTS of an instantiated module.
//!
//! * §7.2.1 / §6.7: `wire s_t w = q;` inside a submodule — member reads
//!   (`w.cnt`, `w.data[i][7:4]`) read the member, from inside the instance and
//!   hierarchically. They read x while the whole net held the right value;
//!   the same declaration at the top level always worked.
//! * §7.4.2 / §23.3.3: element writes into a packed array nested two levels
//!   inside a struct-typed OUTPUT port (`b.s.a[i]`), blocking or NBA,
//!   with or without a preceding whole-port NBA in the same process, land in
//!   the port. They were dropped, while a whole-port or one-level member write
//!   (`b.s.cnt`) worked.
use xezim::simulate;

fn bit(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("t.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", name))
}

#[test]
fn struct_net_member_in_submodule() {
    let src = r#"
package p;
  typedef struct packed { logic [2:0] cnt; logic [3:0][159:0] data; } rec_t;
endpackage
module sub import p::*; (input logic clk, input rec_t din, input logic dv,
                         output logic [2:0] cnt_o, output logic [3:0] op_o);
  rec_t c_q;
  always_ff @(posedge clk) if (dv) c_q <= din;
  wire rec_t alias_w = c_q;
  always_comb begin cnt_o = alias_w.cnt; op_o = alias_w.data[0][7:4]; end
endmodule
module t;
  import p::*;
  logic clk = 0, dv = 0; rec_t din; logic [2:0] cnt_o; logic [3:0] op_o;
  logic [2:0] hier_cnt;
  sub u(.*);
  initial begin
    din = '0; din.cnt = 3'd5; din.data[0] = 160'h1020;
    dv = 1; #1 clk = 1; #1 hier_cnt = u.alias_w.cnt;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(bit(&sim, "cnt_o"), 5);
    assert_eq!(bit(&sim, "op_o"), 2);
    assert_eq!(bit(&sim, "hier_cnt"), 5);
}

#[test]
fn nested_element_write_to_struct_output_port() {
    let src = r#"
package p;
  typedef struct packed { logic [2:0] cnt; logic [3:0][3:0] a; } rec_t;
  typedef struct packed { logic flag; rec_t s; } outer_t;
endpackage
module sub import p::*; (input logic clk, rst_n, input outer_t src, input logic v, output outer_t b);
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) b <= '0;
    else if (v) begin
      b <= src;                                             // whole-port NBA first
      for (int dst = 0; dst < 4; dst++) b.s.a[dst] <= 4'(dst + 8);
    end
  end
endmodule
module sub_blk import p::*; (input logic go, output outer_t b);
  always @(posedge go) begin
    b = '0;
    for (int dst = 0; dst < 4; dst++) b.s.a[dst] = 4'(dst + 4);
  end
endmodule
module t;
  import p::*;
  logic clk = 0, rst_n = 0, v = 0, go = 0; outer_t src, b, bb;
  sub u(.clk, .rst_n, .src, .v, .b);
  sub_blk ub(.go, .b(bb));
  initial begin
    src = '0; src.s.a = 16'h0030; src.flag = 1;
    #1 rst_n = 1; v = 1; go = 1;
    #1 clk = 1;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let b = bit(&sim, "b");
    assert_eq!(
        b & 0xFFFF,
        0xBA98,
        "per-element NBAs after the whole-port NBA"
    );
    assert_eq!((b >> 19) & 1, 1, "flag from the whole-port NBA");
    assert_eq!(bit(&sim, "bb") & 0xFFFF, 0x7654, "blocking element writes");
}

/// §10.4.2 / §7.2: an NBA to a MEMBER of an element of an unpacked array of
/// unpacked structs, inside an instance (`arr[1].value <= …`). The element
/// type was taken for the member, so the leaf was treated as a struct, its
/// "leaf" path appended `.value` again and the NBA re-issued itself without
/// end (`arr[1].value.value…`) until memory ran out. At the top level it was
/// correct.
#[test]
fn nba_to_member_of_unpacked_struct_array_element_in_instance() {
    let src = r#"
module sub (input logic clk);
  typedef struct { logic [31:0] value; } s_t;
  s_t arr [2];
  typedef struct { struct { struct { logic [31:0] value; } val; } w[4]; } fs_t;
  fs_t fs;
  always @(posedge clk) arr[1].value <= 32'd7;
  for (genvar i = 0; i < 4; i++) begin : g
    always @(posedge clk) fs.w[i].val.value <= 32'(i + 1);
  end
endmodule
module t;
  logic clk = 0; logic [31:0] v1, w2;
  sub u(.clk);
  initial begin #1 clk = 1; #1 v1 = u.arr[1].value; w2 = u.fs.w[2].val.value; end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(bit(&sim, "v1"), 7);
    assert_eq!(bit(&sim, "w2"), 3);
}

/// §7.2 / §23.3.3: an unpacked-struct variable inside an INSTANCE, driven by
/// a child's unpacked-struct output port. The port assign must be split per
/// member leaf like a top-level one; it was never split, because the
/// variable has no signal of its own name, and every leaf stayed x.
#[test]
fn unpacked_struct_output_port_into_instance_variable() {
    let src = r#"
package p;
  typedef struct { logic value; } c_t;
  typedef struct { c_t en; } b_t;
  typedef struct { logic [4:0] value; logic m; } d_t;
  typedef struct { d_t o1; } e_t;
  typedef struct { b_t f0; e_t f1; } out_t;
endpackage
module leaf (input wire clk, input logic wr, output p::out_t o);
  logic q0 = 0;
  always_ff @(posedge clk) if (wr) q0 <= 1'b1;
  assign o.f0.en.value = q0;
  assign o.f1.o1.value = 5'd3;
  assign o.f1.o1.m = wr;
endmodule
module mid (input wire clk, input logic wr, output logic o0, output logic [4:0] o1);
  p::out_t v;
  leaf u_leaf (.clk, .wr, .o(v));
  assign o0 = v.f0.en.value;
  assign o1 = v.f1.o1.value;
endmodule
module t;
  logic clk = 0, wr = 0, o0; logic [4:0] o1;
  mid dut (.clk, .wr, .o0, .o1);
  initial begin #1 wr = 1; #1 clk = 1; #1 wr = 0; end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(bit(&sim, "o0"), 1);
    assert_eq!(bit(&sim, "o1"), 3);
}
