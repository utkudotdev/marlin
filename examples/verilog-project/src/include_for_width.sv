`include "include_me.sv"

module main (
    input  [`DEFINED_SIZE-1:0] inp,
    output [`DEFINED_SIZE-1:0] out
);
    assign out = inp;
endmodule
