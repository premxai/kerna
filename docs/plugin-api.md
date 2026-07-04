# Plugin Development

Kerna uses the **Model Context Protocol (MCP)** to communicate with external tools. 

If you want to give Kerna access to a new system (like AWS, GitHub, or your internal CRM), you don't need to write Rust code. You simply write a standard MCP server in Python, Node, or Go, and attach it to Kerna via the config.

## Adding a Plugin

You can quickly generate boilerplate for a new plugin:

```bash
kerna plugins add weather
```

This appends the following to your `kerna.toml`:

```toml
[[plugins]]
name = "weather"
command = ""
args = []
enabled = false
capabilities = []
allowed_paths = []
approval_required = []
```

## Creating the Server (Python Example)

Using the official MCP Python SDK:

```python
from mcp.server.fastmcp import FastMCP

mcp = FastMCP("weather")

@mcp.tool()
def get_weather(location: str) -> str:
    """Get the current weather for a location."""
    return f"The weather in {location} is 72F and sunny."

if __name__ == "__main__":
    mcp.run_stdio_async()
```

Update your `kerna.toml` to point to it:
```toml
command = "python"
args = ["weather.py"]
enabled = true
capabilities = ["weather.read"]
```

Now Kerna can check the weather!
