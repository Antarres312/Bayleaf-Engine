// Headless Bayleaf Engine: run commands and batch jobs. The same subcommands, options and output as
// the Rust bayleaf-cli (rust/apps/bayleaf-cli); `mcp` comes with the automation port.

using Bayleaf.Cli;

return Cli.Run(args, Console.In, Console.Out, Console.Error);
