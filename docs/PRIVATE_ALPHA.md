# Kerna Private Alpha Guide

Welcome to the Kerna Private Alpha! Kerna is a runtime trust layer for agentic execution, enforcing budgets, policies, and observable boundaries.

## The Goal

Your goal in this private alpha is to break the system. We want you to try to make Kerna do something it shouldn't, or find friction points where it's too difficult to do something it should. 

## The Rules of Engagement

1. **Test the Boundaries**: Try to execute tools that are not in a plugin's manifest. Try to write to paths outside of the allowed list. 
2. **Push the Budgets**: Try to trigger infinite loops, prompt injections, or context window exhaustion. See if `max_tool_calls` or `max_cost_usd` catches you.
3. **Examine the Traces**: Look closely at `kerna trace last` and `kerna inspect <task_id>`. Is the reasoning clear? Are the policy rejections accurately reflected?
4. **Use Your Own Tools**: Bring your own Python, JS, or Go MCP servers. Does Kerna proxy them correctly? Does the Risk Card accurately assess them?

## Reporting Feedback

Please report your findings directly in our dedicated Slack channel or GitHub repository. Include the exact output of `kerna export <task_id> --format md` whenever possible.

Thank you for helping us harden the future of agentic runtimes!
