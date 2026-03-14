"""Abstract base class for optimizer rules."""

from __future__ import annotations

from abc import ABC, abstractmethod

from smelt_sdk.types import (
    ModelInfo,
    Opportunity,
    ReplaceWithPlan,
    SelectAnalysis,
    SetIncremental,
)


class OptimizerRule(ABC):
    """Base class for smelt optimizer rules written in Python.

    Subclass this and implement ``name``, ``detect``, and ``rewrite``.
    Register via a ``[project.entry-points."smelt.optimizer_rules"]``
    entry in your package's ``pyproject.toml``.
    """

    @abstractmethod
    def name(self) -> str:
        """Unique name for this rule."""
        ...

    @abstractmethod
    def detect(
        self,
        model: ModelInfo,
        analysis: SelectAnalysis | None,
    ) -> Opportunity | None:
        """Return an Opportunity if this rule applies, else None."""
        ...

    @abstractmethod
    def rewrite(
        self,
        model: ModelInfo,
        analysis: SelectAnalysis | None,
    ) -> ReplaceWithPlan | SetIncremental | None:
        """Return a transformation, or None if rewrite is not possible."""
        ...
