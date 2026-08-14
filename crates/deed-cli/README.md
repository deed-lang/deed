# Deed

Deed is a contract-first language for code that machines write and humans
review. The toolchain checks types, effect rows, capabilities, contracts, and
before-and-after patch receipts.

```console
cargo install deed-lang
deed new greeting
deed check greeting
deed test greeting
```

The package is named `deed-lang`; it installs the `deed` binary. Run the local
Model Context Protocol server over stdio with:

```console
deed mcp
```

- [Repository](https://github.com/deed-lang/deed)
- [Agent guide](https://deed-lang.github.io/agents/)
- MCP Registry name: `mcp-name: io.github.deed-lang/deed`