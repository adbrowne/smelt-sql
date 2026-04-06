# smelt LSP Features

Visual showcase of smelt's Language Server Protocol features. All features work in real time as you edit — no build step required.

## Contents

- [Real-Time Diagnostics](#real-time-diagnostics)
- [Go-to-Definition](#go-to-definition)
- [Hover Information](#hover-information)
- [Code Completion](#code-completion)
- [Find References](#find-references)
- [Rename Refactoring](#rename-refactoring)
- [Code Actions & Quick Fixes](#code-actions-quick-fixes)

## Real-Time Diagnostics

Catch errors the moment you make them. smelt validates SQL continuously: undefined model references, undeclared columns, type mismatches, and parse errors all surface as you type.

![Real-Time Diagnostics](media/diagnostics/typo-caught-instantly.gif)

![clean pipeline](media/diagnostics/01-clean-pipeline.png)

![undeclared column](media/diagnostics/04-undeclared-column.png)

<details>
<summary>All media in this section</summary>

![02-typo-error-visible.png](media/diagnostics/02-typo-error-visible.png)

![02-typo-hover-tooltip.png](media/diagnostics/02-typo-hover-tooltip.png)

![03-type-mismatch.png](media/diagnostics/03-type-mismatch.png)

![04-undeclared-column-detail.png](media/diagnostics/04-undeclared-column-detail.png)

![04-undeclared-column-hover.png](media/diagnostics/04-undeclared-column-hover.png)

</details>

## Go-to-Definition

Trace data lineage by jumping directly to definitions — from a smelt.ref() call to the upstream model, from a column reference to where it's defined, or from a CTE usage to its definition.

![Go-to-Definition](media/goto-definition/trace-pipeline.gif)

![cte definition jump](media/goto-definition/05-cte-definition-jump.png)

<details>
<summary>All media in this section</summary>

![01-start-daily-revenue.png](media/goto-definition/01-start-daily-revenue.png)

![02-jumped-to-stg-events.png](media/goto-definition/02-jumped-to-stg-events.png)

![03-jumped-to-sources-yml.png](media/goto-definition/03-jumped-to-sources-yml.png)

![05-cte-definition-jump-full.png](media/goto-definition/05-cte-definition-jump-full.png)

![06-source-before-jump.png](media/goto-definition/06-source-before-jump.png)

![06-source-definition-landing-full.png](media/goto-definition/06-source-definition-landing-full.png)

![06-source-definition-landing.png](media/goto-definition/06-source-definition-landing.png)

</details>

## Hover Information

Hover over any model reference, source, or CTE to see its full schema — column names, types, and where the data comes from.

![model schema on hover editor](media/hover/01-model-schema-on-hover-editor.png)

![upstream schema lineage](media/hover/02-upstream-schema-lineage.png)

![source schema on hover editor](media/hover/03-source-schema-on-hover-editor.png)

<details>
<summary>All media in this section</summary>

![01-model-schema-on-hover.png](media/hover/01-model-schema-on-hover.png)

![03-source-schema-on-hover.png](media/hover/03-source-schema-on-hover.png)

</details>

## Code Completion

Build queries faster with context-aware completions. smelt suggests model names, source names, and column names based on upstream schemas.

![Code Completion](media/completion/build-query-with-completions.gif)

![model name completions editor](media/completion/01-model-name-completions-editor.png)

<details>
<summary>All media in this section</summary>

![01-model-name-completions.png](media/completion/01-model-name-completions.png)

![02-source-completions-editor.png](media/completion/02-source-completions-editor.png)

![02-source-completions.png](media/completion/02-source-completions.png)

</details>

## Find References

Answer "who uses this model?" with a single keystroke. Find References shows every downstream consumer of a model or every usage of a CTE within a file.

![find model consumers editor](media/references/01-find-model-consumers-editor.png)

![find cte references editor](media/references/02-find-cte-references-editor.png)

<details>
<summary>All media in this section</summary>

![01-find-model-consumers.png](media/references/01-find-model-consumers.png)

![02-find-cte-references.png](media/references/02-find-cte-references.png)

</details>

## Rename Refactoring

Rename a model and have every reference across the project update automatically. The LSP shows a preview of all changes before applying them.

![Rename Refactoring](media/rename/rename-model-across-project.gif)

![rename preview editor](media/rename/01-rename-preview-editor.png)

<details>
<summary>All media in this section</summary>

![01-rename-preview.png](media/rename/01-rename-preview.png)

</details>

## Code Actions & Quick Fixes

When smelt detects an error, it often suggests a fix. Reference a model that doesn't exist yet? smelt offers to create the SQL file for you.

![Code Actions & Quick Fixes](media/code-actions/create-model-from-ref.gif)

![create model quickfix editor](media/code-actions/01-create-model-quickfix-editor.png)

<details>
<summary>All media in this section</summary>

![01-create-model-quickfix.png](media/code-actions/01-create-model-quickfix.png)

![03-undeclared-column-quickfix-editor.png](media/code-actions/03-undeclared-column-quickfix-editor.png)

![03-undeclared-column-quickfix.png](media/code-actions/03-undeclared-column-quickfix.png)

</details>
