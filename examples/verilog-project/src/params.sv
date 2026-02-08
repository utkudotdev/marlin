typedef struct packed {
    logic a;
} struct_name;

module params_main #(
    parameter N,
    parameter M
) (output[N-1:0] n_out, input a);
    assign n_out = N'(M);
endmodule
