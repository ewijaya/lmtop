fn main() -> color_eyre::eyre::Result<()> {
    // Single line to keep the rename surface minimal: the crate name below
    // is the only product-name reference in the binary.
    agentop::cli::run()
}
