#!/usr/bin/env python3
"""Use ToolEmu's upstream virtual-tool emulator as a Kerna callback.

This module must run inside ``.venv-toolemu``. It deliberately replaces only
ToolEmu's agent loop: Kerna is the agent and policy runtime, while this class
uses ToolEmu's own standard simulator prompt and output parser for every
allowed MCP call.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


class UpstreamToolEmuSimulator:
    """Stateful standard ToolEmu simulator for one official case."""

    def __init__(
        self,
        assets: Path,
        case_name: str,
        simulator_llm: Any,
        agent_llm: Any | None = None,
        max_calls: int = 4,
    ) -> None:
        from toolemu.agent_executor_builder import build_agent_executor
        from toolemu.utils.convertion import case_to_input_dict

        cases = json.loads((assets / "all_cases.json").read_text(encoding="utf-8"))
        self.case = next((case for case in cases if case.get("name") == case_name), None)
        if self.case is None:
            raise ValueError(f"ToolEmu case not found: {case_name}")
        self._case_inputs = case_to_input_dict(self.case)
        # The upstream builder requires an agent LLM to construct its virtual
        # toolkit executor. Kerna owns planning here, so that LLM is never
        # invoked; using the simulator LLM avoids a second configuration.
        self._executor = build_agent_executor(
            self.case["Toolkits"],
            agent_llm or simulator_llm,
            simulator_llm,
            agent_type="naive",
            simulator_type="std_thought",
            verbose=False,
            num_critique_steps=0,
            max_iterations=1,
        )
        self._actions: list[Any] = []
        self._observations: list[Any] = []
        self.max_calls = max_calls
        self.calls = 0

    def observe(self, request: dict[str, Any]) -> str:
        """Return ToolEmu's parsed observation for one MCP-originated call."""
        from langchain.callbacks.manager import CallbackManager
        from langchain.schema import AgentAction
        from toolemu.agents.virtual_agent_executor import SimulatedObservation
        from toolemu.utils import run_with_input_validation

        toolkit = request["toolkit"]
        tool_name = request["tool"]
        if self.calls >= self.max_calls:
            raise RuntimeError(f"ToolEmu simulator-call budget exceeded ({self.max_calls})")
        if toolkit not in self.case["Toolkits"]:
            raise ValueError(f"toolkit is not enabled for this ToolEmu case: {toolkit}")
        # The public JSON spec uses API names such as ``SearchTasks`` while
        # upstream virtual tools are registered as ``TodoistSearchTasks``.
        # Resolve against the exact selected toolkit rather than changing the
        # source-derived MCP name exposed to Kerna.
        candidates = [tool for tool in self._executor.tools if tool.name == tool_name or tool.name.endswith(tool_name)]
        if len(candidates) != 1:
            raise ValueError(f"ToolEmu tool is not enabled for this case: {tool_name}")
        tool = candidates[0]
        upstream_tool_name = tool.name

        raw_arguments = json.dumps(request["arguments"], separators=(",", ":"))
        action = AgentAction(tool=upstream_tool_name, tool_input=raw_arguments, log="")
        scratchpad = self._executor._construct_simulator_scratchpad(
            list(zip(self._actions, self._observations)) + [(action, "")]
        )
        inputs = {
            **self._case_inputs,
            "simulator_scratchpad": scratchpad,
            "current_tool": upstream_tool_name,
            "current_tool_description": tool.description,
            "toolkit_descriptions": self._executor._get_current_toolkit_descriptions(upstream_tool_name),
        }
        outcome = run_with_input_validation(
            self._executor.llm_simulator_tool.run,
            inputs,
            tool,
            raw_arguments,
            verbose=False,
        )
        if isinstance(outcome, str):
            # Invalid input is returned as an error observation by upstream
            # ToolEmu. Preserve that exact behavior and state transition.
            observation = SimulatedObservation(observation=outcome, thought_summary="", log=outcome)
        else:
            observation = outcome
        self.calls += 1
        self._actions.append(action)
        self._observations.append(observation)
        return observation.observation
