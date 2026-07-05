"""
Weather Plugin (MCP Server)

To run this, first add it to your kerna.toml:

[[mcp_servers]]
name = "weather"
command = "python"
args = ["examples/plugins/weather_plugin.py"]
enabled = true
"""

from mcp.server.fastmcp import FastMCP
import httpx

mcp = FastMCP("weather")

@mcp.tool()
async def get_weather(location: str) -> str:
    """Get the current weather for a location."""
    # A real implementation would call an API like OpenWeatherMap
    return f"The weather in {location} is 72F and sunny."

if __name__ == "__main__":
    mcp.run_stdio_async()
