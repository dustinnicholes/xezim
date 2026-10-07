//! Width and range handling that only shows up past 64 bits, at a non-zero
//! lower bound, or in a nested aggregate.
//!
//! * §11.4.5: `==`/`!=` compare the WHOLE operands. Equal-width operands
//!   wider than 64 bits differing only above bit 63 compared equal.
//! * §6.20.2 / §26.3: an unpacked-array parameter declared in a PACKAGE reads
//!   back element by element (every element read 0), through a wildcard
//!   import, an explicit import and a scoped reference — its dimension
//!   resolved in the PACKAGE, also when only the array is imported or the
//!   importer declares a parameter of the same name.
//! * §13.4.3 / §7.4: a constant function with an unpacked-array formal, called
//!   with an assignment pattern or an array parameter in a parameter
//!   initializer, iterating with `foreach` — the parameter read 0. An array
//!   actual binds POSITIONALLY in its own declared order, so a `[4:7]` actual
//!   fills a `[0:3]` formal; and a `[7:4]` parameter's pattern fills element 7
//!   first (§10.9.1).
//! * §7.4.1 / §11.5.1: a bit-select write into a vector declared `[5:1]` in a
//!   loop body maps the label through the declared range.
//! * §10.4.2 / §7.4: an NBA to an element of a packed array inside a packed
//!   struct array (`q.e[k].c[5] <= 2'b10`) stores the whole element.
//! * §7.4.2: a compiled NBA to a 2-D unpacked array element wider than 32 bits
//!   stores the whole element.
//! * §6.18 / §23.10: a cast through a typedef declared in a parameterized
//!   module uses THIS instance's layout.
use xezim::simulate;

fn get(sim: &xezim::compiler::Simulator, name: &str) -> xezim_core::value::Value {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("t.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .clone()
}

fn bit(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    get(sim, name)
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", name))
}

#[test]
fn equality_wider_than_64_bits() {
    let src = r#"
module t;
  logic [151:0] a, b2, c; logic [99:0] p, q; logic [64:0] u, v;
  logic ne, eq, ne_shift, ne_100, ne_65, eq_same, ne_ff, eq_ff;
  logic clk = 0;
  always @(posedge clk) begin ne_ff <= (a != c); eq_ff <= (a == c); end
  initial begin
    a = {1'b1, 151'b0}; c = '0; b2 = 152'h1 << 100; p = 100'h1 << 70; q = '0;
    u = '0; v = '0; u[64] = 1;
    ne = (a != c); eq = (a == c); ne_shift = (b2 != c); ne_100 = (p != q);
    ne_65 = (u != v); eq_same = (a == a);
    #1 clk = 1;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    for (n, want) in [
        ("ne", 1),
        ("eq", 0),
        ("ne_shift", 1),
        ("ne_100", 1),
        ("ne_65", 1),
        ("eq_same", 1),
        ("ne_ff", 1),
        ("eq_ff", 0),
    ] {
        assert_eq!(bit(&sim, n), want, "{n}");
    }
}

#[test]
fn package_unpacked_array_parameter() {
    let src = r#"
package p;
  parameter int NG = 4;
  parameter int OFF [NG] = '{3, 17, 40, 61};
  localparam int LA [0:3] = '{5, 6, 7, 8};
endpackage
module t;
  import p::*;
  int e0, e1, e3, l2, scoped, dyn;
  logic [1023:0] wide_v; logic [3:0][31:0] vals;
  always_comb begin
    wide_v = '0;
    for (int g = 0; g < NG; g++) wide_v[OFF[g]*8 +: 32] = vals[g];
  end
  initial begin
    vals = {32'hD3, 32'hD2, 32'hD1, 32'hD0};
    e0 = OFF[0]; e1 = OFF[1]; e3 = OFF[3]; l2 = LA[2]; scoped = p::OFF[1];
    #1 dyn = wide_v[17*8 +: 32];
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(bit(&sim, "e0"), 3);
    assert_eq!(bit(&sim, "e1"), 17);
    assert_eq!(bit(&sim, "e3"), 61);
    assert_eq!(bit(&sim, "l2"), 7);
    assert_eq!(bit(&sim, "scoped"), 17);
    assert_eq!(
        bit(&sim, "dyn"),
        0xD1,
        "element-indexed part-select placed word 1 elsewhere"
    );
}

#[test]
fn package_array_parameter_sized_in_its_package() {
    let src = r#"
package p;
  parameter int N = 4;
  parameter int OFF [N] = '{3, 17, 40, 61};
endpackage
module sub;
  import p::*;
  localparam int N = 2;              // the importer's own N
  int s3;
  initial s3 = OFF[3];
endmodule
module t;
  import p::OFF;                     // N itself is not imported
  int e0, e1, e3, s3;
  sub u();
  initial begin e0 = OFF[0]; e1 = OFF[1]; e3 = OFF[3]; #1 s3 = u.s3; end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(bit(&sim, "e0"), 3);
    assert_eq!(bit(&sim, "e1"), 17);
    assert_eq!(bit(&sim, "e3"), 61);
    assert_eq!(bit(&sim, "s3"), 61, "OFF sized by the importer's N");
}

#[test]
fn constant_function_array_actual_with_offset_range() {
    let src = r#"
module t;
  parameter int B [4:7] = '{1, 2, 3, 4};
  parameter int D [7:4] = '{1, 2, 3, 4};
  function automatic int first(input int v [0:3]); return v[0]; endfunction
  function automatic int digits(input int v [0:3]);
    int acc; acc = 0;
    foreach (v[i]) acc = acc * 10 + v[i];
    return acc;
  endfunction
  localparam int FB = first(B), WB = digits(B), D7 = D[7], D4 = D[4], B4 = B[4];
  int fb, wb, d7, d4, b4;
  initial begin fb = FB; wb = WB; d7 = D7; d4 = D4; b4 = B4; end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(
        bit(&sim, "fb"),
        1,
        "B[4] is the leftmost element, bound to v[0]"
    );
    assert_eq!(bit(&sim, "wb"), 1234);
    assert_eq!(bit(&sim, "b4"), 1);
    assert_eq!(
        bit(&sim, "d7"),
        1,
        "§10.9.1: the first item fills the LEFT bound"
    );
    assert_eq!(bit(&sim, "d4"), 4);
}

#[test]
fn constant_function_with_array_formal() {
    let src = r#"
package p;
  parameter int A = 8, B = 32, C = 4;
  localparam int N = 3;
  parameter int ARR [4] = '{5, 9, 2, 7};
  function automatic int max_of(input int v [N]);
    max_of = 0;
    foreach (v[i]) if (v[i] > max_of) max_of = v[i];
  endfunction
  function automatic int digits(input int v [3:0]);   // descending range
    int acc; acc = 0;
    foreach (v[i]) acc = acc * 10 + v[i];               // walks 3, 2, 1, 0
    return acc;
  endfunction
  function automatic int mx4(input int v [4]);
    mx4 = 0; for (int i = 0; i < 4; i++) if (v[i] > mx4) mx4 = v[i];
  endfunction
  parameter int MAXD = max_of('{A, B, C});
  parameter int LW = $clog2(MAXD + 1);
  parameter int DIG = digits('{1, 2, 3, 4});
  parameter int FROM_ARR = mx4(ARR);
endpackage
module t;
  import p::*;
  logic [LW-1:0] lvl;
  int maxd, lw, bits, dig, from_arr;
  initial begin maxd = MAXD; lw = LW; bits = $bits(lvl); dig = DIG; from_arr = FROM_ARR; end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(bit(&sim, "maxd"), 32);
    assert_eq!(bit(&sim, "lw"), 6);
    assert_eq!(bit(&sim, "bits"), 6);
    assert_eq!(
        bit(&sim, "dig"),
        1234,
        "§7.4.2: v[3]=1 … v[0]=4, walked 3 down to 0"
    );
    assert_eq!(bit(&sim, "from_arr"), 9);
}

#[test]
fn loop_body_local_with_offset_range() {
    let src = r#"
module t;
  logic clk = 0;
  logic [6:0][5:1] st;
  logic [2:0][2:0] out_q;
  logic [5:1] o_var, o_reused;
  logic [4:0] z_reused;
  always_ff @(posedge clk) begin
    for (int e = 0; e < 3; e++) begin
      logic [5:1] sel;
      logic [2:0] acc;
      sel = '0;
      sel[1] = st[4 + e][1];       // RHS depends on the loop variable
      sel[2] = st[3 + e][2];
      acc = '0;
      if (sel[1]) acc |= 3'd1;
      if (sel[2]) acc |= 3'd2;
      out_q[e] <= acc;
    end
    for (int i = 1; i < 2; i++) begin logic [5:1] v; v = '0; v[i] = 1'b1; o_var <= v; end
  end
  always @(posedge clk) begin
    for (int i = 0; i < 1; i++) begin logic [5:1] b; b = '0; b[i+1] = 1'b1; o_reused <= b; end
    for (int i = 0; i < 1; i++) begin logic [4:0] b; b = '0; b[i+1] = 1'b1; z_reused <= b; end
  end
  initial begin
    st = '0; st[4] = 5'b00001;            // one set bit at index 4 -> out_q[0]
    #1 clk = 1;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let r = bit(&sim, "out_q");
    assert_eq!(r & 7, 1, "out_q[0]");
    assert_eq!((r >> 3) & 7, 0, "out_q[1]");
    assert_eq!(
        bit(&sim, "o_var"),
        0b00001,
        "v[1] of [5:1] is storage bit 0"
    );
    assert_eq!(bit(&sim, "o_reused"), 0b00001);
    assert_eq!(
        bit(&sim, "z_reused"),
        0b00010,
        "a same-named zero-based local must not inherit the [5:1] range"
    );
}

#[test]
fn nba_to_packed_element_inside_struct_array() {
    let src = r#"
module t;
  typedef struct packed { logic [5:0][1:0] c; logic f; } el_t;
  typedef struct packed { el_t [2:0] e; logic b; } grp_t;
  grp_t q1, q2, q3, q4; int k;
  initial begin
    q1 = '0; q2 = '0; q3 = '0; q4 = '0; k = 0;
    q1.e[0].c[5] = 2'b10;
    q2.e[k].c[5] = 2'b10;
    q3.e[0].c[5] <= 2'b10;
    q4.e[k].c[5] <= 2'b10;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    // e[0] st at bit 1 (after `b`); within the e `f` is bit 0, so
    // c[5] is e bits [12:11], i.e. bits [13:12] of the struct.
    for n in ["q1", "q2", "q3", "q4"] {
        assert_eq!((bit(&sim, n) >> 12) & 3, 0b10, "{n}.e[0].c[5]");
    }
}

#[test]
fn wide_element_of_2d_array_in_a_compiled_block() {
    let src = r#"
module t;
  logic clk = 0; logic [63:0] m [3][4]; logic [99:0] w [2][2];
  logic [63:0] d; logic [99:0] dw; int bk; logic [63:0] r; logic [99:0] rw;
  always @(posedge clk) begin m[bk][2] <= d; w[bk][1] <= dw; end
  initial begin
    d = 64'hDEAD_BEEF_0123_4567; dw = {36'h9_8765_4321, 64'hDEAD_BEEF_0123_4567}; bk = 1;
    #1 clk = 1; #1 r = m[1][2]; rw = w[1][1];
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(bit(&sim, "r"), 0xDEAD_BEEF_0123_4567);
    let rw = get(&sim, "rw");
    assert_eq!(
        rw.to_hex_string().trim_start_matches('0'),
        "987654321deadbeef01234567"
    );
}

#[test]
fn typedef_cast_uses_the_instance_layout() {
    let src = r#"
module q #(parameter int DEPTH = 8, parameter int NB = 3,
           parameter int MW = $clog2((DEPTH+NB-1)/NB) < 1 ? 1 : $clog2((DEPTH+NB-1)/NB),
           parameter int LW = NB == 1 ? 1 : $clog2(NB),
           parameter int PTR_W = 1 + MW + LW)
  (input logic clk, input logic [PTR_W-1:0] din, output logic [PTR_W-1:0] q_cast);
  typedef struct packed { logic hi; logic [MW-1:0] md; logic [LW-1:0] lo; } rec_t;
  rec_t c_q, c_blk;
  always @(posedge clk) c_q <= rec_t'(din);
  always @(din) c_blk = rec_t'(din);
  assign q_cast = c_q;
endmodule
module t;
  logic clk = 0;
  logic [4:0] t5, c5; logic [6:0] t7, c7;
  q #(.DEPTH(8))  qa(.clk, .din(t5), .q_cast(c5));   // PTR_W = 5
  q #(.DEPTH(32)) qb(.clk, .din(t7), .q_cast(c7));   // PTR_W = 7
  initial begin t5 = 5'b10000; t7 = 7'b1000000; #1 clk = 1; end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(bit(&sim, "c5"), 0b10000);
    assert_eq!(
        bit(&sim, "c7"),
        0b1000000,
        "the 7-bit instance cast through the 5-bit layout"
    );
}
