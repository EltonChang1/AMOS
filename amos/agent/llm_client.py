from __future__ import annotations


class MockLLMClient:
    """Placeholder interface for later LLM integration behind AMOS verification."""

    def complete(self, prompt: str) -> str:
        return prompt
