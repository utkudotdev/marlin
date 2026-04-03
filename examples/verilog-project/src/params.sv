module params_main #(
    parameter N,
    parameter M,
    parameter K
) (
    output [N-1:0] n_out,
    input  [M-1:0] m_in
);
    assign n_out = N'(K) + N'(m_in);
endmodule
