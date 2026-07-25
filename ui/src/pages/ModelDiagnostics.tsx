import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { fetchModel, fetchModelDiagnostics } from '../api'
import { ColumnsTable } from '../components/ModelDetail'
import { SqlViewer } from '../components/SqlViewer'
import type {
  Admissibility,
  BatchSafetyInfo,
  BoundResult,
  ColumnComparability,
  ColumnDeterminism,
  ColumnDiscriminant,
  DerivedFd,
  PlanCellDiagnostics,
  PropertySet,
  RelationContractView,
  RowIdentity,
  Technique,
  TechniquePreview,
} from '../types'

interface ModelDiagnosticsProps {
  model: string
  onClose: () => void
}

const TECHNIQUE_LABELS: Record<Technique, string> = {
  DeleteInsert: 'Delete + Insert (region recompute)',
  KeyedFold: 'Keyed Fold',
  ColumnScopedMerge: 'Column-Scoped Merge',
  InPlaceUpdate: 'In-Place Update',
}

function formatRowIdentity(identity: RowIdentity): string {
  if (identity === 'WholeRow') return 'whole row (no key)'
  return identity.Key.join(', ')
}

function BatchSafetyBadge({ safety }: { safety: BatchSafetyInfo }) {
  const colors = {
    fully_batch_safe: 'bg-green-100 text-green-700',
    bounded_safe: 'bg-yellow-100 text-yellow-700',
    per_partition_only: 'bg-red-100 text-red-700',
  }
  const labels = {
    fully_batch_safe: 'Batch Safe',
    bounded_safe: `Bounded (${safety.max_chunk_days}d chunks, ${safety.context_days}d context)`,
    per_partition_only: 'Per-Partition Only',
  }
  const colorClass = colors[safety.level as keyof typeof colors] ?? 'bg-gray-100 text-gray-600'
  const label = labels[safety.level as keyof typeof labels] ?? safety.level

  return (
    <span className={`text-xs px-2 py-0.5 rounded font-medium ${colorClass}`} title={safety.reason ?? undefined}>
      {label}
    </span>
  )
}

/** Renders the API response's admissibility verdict verbatim — no
 * client-side re-derivation (`docs/specs/ui_model_diagnostics.md`
 * §Semantics "Thin-consumer boundary" applies to the frontend too).
 */
function AdmissibilityBadge({ admissibility }: { admissibility: Admissibility }) {
  if (admissibility.verdict === 'admitted') {
    return (
      <span className="text-xs px-2 py-0.5 rounded font-medium bg-green-100 text-green-700">
        Admitted
      </span>
    )
  }
  if (admissibility.verdict === 'interchangeable_alternative') {
    return (
      <span className="text-xs px-2 py-0.5 rounded font-medium bg-blue-100 text-blue-700">
        Interchangeable Alternative
      </span>
    )
  }
  return (
    <span
      className="text-xs px-2 py-0.5 rounded font-medium bg-gray-200 text-gray-600"
      title={admissibility.reason}
    >
      Not Applicable — {admissibility.reason}
    </span>
  )
}

function RelationContractBlock({ title, contract }: { title: string; contract: RelationContractView }) {
  return (
    <div className="text-xs bg-gray-50 rounded p-2 space-y-1">
      <div className="font-semibold text-gray-600">{title}</div>
      {contract.clock && (
        <div className="flex justify-between">
          <span className="text-gray-500">Clock</span>
          <span className="font-mono text-gray-700">
            {contract.clock.event_time_column} / {contract.clock.partition_column} ({contract.clock.granularity})
          </span>
        </div>
      )}
      {contract.identity && contract.identity.length > 0 && (
        <div className="flex justify-between">
          <span className="text-gray-500">Identity</span>
          <span className="font-mono text-gray-700">{contract.identity.join(', ')}</span>
        </div>
      )}
      {contract.derived_grain && (
        <div className="flex justify-between">
          <span className="text-gray-500">Derived Grain</span>
          <span className="font-mono text-gray-700">{contract.derived_grain}</span>
        </div>
      )}
      {!contract.clock && !contract.identity && !contract.derived_grain && (
        <div className="text-gray-400">no contract</div>
      )}
    </div>
  )
}

function BoundBadge({ source, bound }: { source: string; bound: BoundResult }) {
  let text: string
  if (bound.type === 'bounded') {
    text = `${source}: ${bound.source_partition_col} [${bound.before}, ${bound.after})`
  } else if (bound.type === 'unbounded') {
    text = `${source}: unbounded`
  } else {
    text = `${source}: not derivable`
  }
  return (
    <span className="text-xs px-2 py-0.5 rounded bg-gray-100 text-gray-600 font-mono">{text}</span>
  )
}

function PropertiesSection({ properties }: { properties: PropertySet }) {
  return (
    <section className="mb-6">
      <h2 className="text-sm font-semibold text-gray-900 mb-2">Properties</h2>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3 mb-3">
        <div className="text-xs bg-gray-50 rounded p-2">
          <div className="font-semibold text-gray-600 mb-1">Grain</div>
          {properties.grain.keys.length === 0 && <div className="text-gray-400">no grain derived</div>}
          {properties.grain.keys.map((keySet, i) => (
            <div key={i} className="font-mono text-gray-700">
              {keySet.join(', ')}
            </div>
          ))}
        </div>

        <div className="text-xs bg-gray-50 rounded p-2">
          <div className="font-semibold text-gray-600 mb-1">Row Identity</div>
          <div className="font-mono text-gray-700">{formatRowIdentity(properties.row_identity.identity)}</div>
          {properties.row_identity.proven_mismatch && properties.row_identity.proven_mismatch.length > 0 && (
            <div className="text-red-600 mt-1">
              Proven mismatch: {properties.row_identity.proven_mismatch.join(', ')}
            </div>
          )}
        </div>
      </div>

      {properties.functional_dependencies.length > 0 && (
        <div className="text-xs mb-3">
          <div className="font-semibold text-gray-600 mb-1">Functional Dependencies</div>
          <ul className="space-y-0.5">
            {properties.functional_dependencies.map((fd: DerivedFd, i: number) => (
              <li key={i} className="font-mono text-gray-700">
                {fd.key.join(', ')} → {fd.determines}
              </li>
            ))}
          </ul>
        </div>
      )}

      <div className="border border-gray-200 rounded overflow-hidden mb-3">
        <table className="w-full text-xs">
          <thead>
            <tr className="bg-gray-50 border-b border-gray-200">
              <th className="text-left px-2 py-1 font-medium text-gray-600">Column</th>
              <th className="text-left px-2 py-1 font-medium text-gray-600">Determinism</th>
              <th className="text-left px-2 py-1 font-medium text-gray-600">Comparability</th>
              <th className="text-left px-2 py-1 font-medium text-gray-600">Discriminants</th>
            </tr>
          </thead>
          <tbody>
            {properties.columns.map((column) => {
              const determinism = properties.determinism.find((d: ColumnDeterminism) => d.output === column)
              const comparability = properties.comparability.find((c: ColumnComparability) => c.output === column)
              const discriminant = properties.discriminants.find((d: ColumnDiscriminant) => d.output === column)
              return (
                <tr key={column} className="border-b border-gray-100 last:border-0">
                  <td className="px-2 py-1 font-mono text-gray-900">{column}</td>
                  <td className="px-2 py-1 text-gray-600">{determinism?.level ?? '-'}</td>
                  <td className="px-2 py-1 text-gray-600">{comparability?.comparability ?? '-'}</td>
                  <td className="px-2 py-1 text-gray-500 font-mono text-[11px]">
                    {discriminant
                      ? `monoid=${discriminant.discriminants.is_monoid} inverse=${discriminant.discriminants.needs_inverse} decomposable=${discriminant.discriminants.decomposable} monotone=${discriminant.discriminants.monotone}`
                      : '-'}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      <div className="flex gap-2 flex-wrap mb-3">
        {properties.has_set_op_barrier && (
          <span className="text-xs px-2 py-0.5 rounded bg-yellow-100 text-yellow-700">set-op barrier</span>
        )}
        {properties.has_fan_out_join && (
          <span className="text-xs px-2 py-0.5 rounded bg-yellow-100 text-yellow-700">fan-out join</span>
        )}
        {properties.literal_columns.length > 0 && (
          <span className="text-xs px-2 py-0.5 rounded bg-gray-100 text-gray-600 font-mono">
            literals: {properties.literal_columns.map(([col, val]) => `${col}=${val}`).join(', ')}
          </span>
        )}
      </div>

      {Object.keys(properties.source_bounds).length > 0 && (
        <div className="text-xs">
          <div className="font-semibold text-gray-600 mb-1">Source Bounds</div>
          <div className="flex gap-1.5 flex-wrap">
            {Object.entries(properties.source_bounds).map(([source, bound]) => (
              <BoundBadge key={source} source={source} bound={bound} />
            ))}
          </div>
        </div>
      )}
    </section>
  )
}

function TechniquePreviewBlock({
  cell,
  removeComments,
}: {
  cell: PlanCellDiagnostics
  removeComments: boolean
}) {
  const [selectedTechnique, setSelectedTechnique] = useState<Technique>(cell.admitted_technique)

  const selectedPreview: TechniquePreview | undefined = cell.technique_previews.find(
    (p) => p.technique === selectedTechnique
  )

  return (
    <div className="border border-gray-200 rounded p-3 mb-3">
      <div className="flex items-center justify-between mb-2 flex-wrap gap-2">
        <div className="text-sm font-medium text-gray-900">{cell.group}</div>
        <div className="flex gap-2 text-xs text-gray-500">
          <span>trigger: <span className="font-mono text-gray-700">{cell.trigger}</span></span>
          <span>corner: <span className="font-mono text-gray-700">{cell.corner}</span></span>
          <span>row identity: <span className="font-mono text-gray-700">{formatRowIdentity(cell.row_identity)}</span></span>
        </div>
      </div>

      <div className="flex items-center gap-2 mb-2 flex-wrap" role="tablist" aria-label={`${cell.group} technique picker`}>
        {cell.technique_previews.map((preview) => (
          <button
            key={preview.technique}
            role="tab"
            aria-selected={preview.technique === selectedTechnique}
            onClick={() => setSelectedTechnique(preview.technique)}
            className={`text-xs px-2 py-1 rounded border transition-colors ${
              preview.technique === selectedTechnique
                ? 'bg-gray-900 text-white border-gray-900'
                : 'bg-white text-gray-600 border-gray-200 hover:border-gray-400'
            }`}
          >
            {TECHNIQUE_LABELS[preview.technique]}
            {preview.technique === cell.admitted_technique && ' *'}
          </button>
        ))}
      </div>

      {selectedPreview && (
        <div>
          <div className="mb-2">
            <AdmissibilityBadge admissibility={selectedPreview.admissibility} />
          </div>
          <div className="space-y-2">
            {selectedPreview.statements.length === 0 && (
              <div className="text-xs text-gray-400">no statements for this technique</div>
            )}
            {selectedPreview.statements.map((statement, i) => (
              <SqlViewer key={i} sql={statement.sql} removeComments={removeComments} />
            ))}
          </div>
        </div>
      )}
    </div>
  )
}

/**
 * Full-screen per-model diagnostics page (`docs/specs/
 * ui_model_diagnostics.md` §Surface "UI page"): overview (superset of
 * `ModelDetail`), the full property set, and the maintenance plan with a
 * technique picker + admissibility badge per cell. Follows the
 * `RunPlanner`/`RunHistory` full-screen-page pattern — its own `useQuery`
 * calls and its own scroll container.
 */
export function ModelDiagnostics({ model, onClose }: ModelDiagnosticsProps) {
  const [removeComments, setRemoveComments] = useState(false)

  const { data: detail, isLoading: detailLoading, error: detailError } = useQuery({
    queryKey: ['model', model],
    queryFn: () => fetchModel(model),
  })

  const { data: diagnostics, isLoading: diagnosticsLoading, error: diagnosticsError } = useQuery({
    queryKey: ['modelDiagnostics', model],
    queryFn: () => fetchModelDiagnostics(model),
  })

  const isLoading = detailLoading || diagnosticsLoading
  const error = detailError ?? diagnosticsError

  return (
    <div className="h-full overflow-y-auto w-full">
      <div className="max-w-5xl mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-xl font-semibold text-gray-900">Diagnostics: {model}</h1>
          <div className="flex items-center gap-4">
            <label className="flex items-center gap-2 text-sm text-gray-700">
              <input
                type="checkbox"
                checked={removeComments}
                onChange={(e) => setRemoveComments(e.target.checked)}
                className="rounded border-gray-300"
              />
              Remove comments
            </label>
            <button
              onClick={onClose}
              className="text-sm text-gray-500 hover:text-gray-800 border border-gray-300 rounded px-3 py-1"
            >
              Close
            </button>
          </div>
        </div>

        {isLoading && <p className="text-gray-500 text-sm">Loading diagnostics...</p>}

        {error && (
          <div className="bg-red-50 border border-red-200 text-red-700 rounded p-3 text-sm mb-6">
            {(error as Error).message}
          </div>
        )}

        {detail && (
          <section className="mb-6">
            <h2 className="text-sm font-semibold text-gray-900 mb-2">Overview</h2>
            <p className="text-xs text-gray-500 mb-2 font-mono">{detail.path}</p>
            {detail.description && <p className="text-sm text-gray-700 mb-2">{detail.description}</p>}

            <div className="flex gap-2 mb-3 flex-wrap">
              {detail.materialization && (
                <span className="text-xs bg-amber-100 text-amber-700 px-2 py-0.5 rounded font-medium">
                  {detail.materialization}
                </span>
              )}
              {detail.batch_safety && <BatchSafetyBadge safety={detail.batch_safety} />}
              {detail.tags.map((tag) => (
                <span key={tag} className="text-xs bg-gray-100 text-gray-600 px-2 py-0.5 rounded">
                  {tag}
                </span>
              ))}
            </div>

            {detail.owner && <p className="text-xs text-gray-500 mb-3">Owner: {detail.owner}</p>}

            {detail.incremental && (
              <div className="mb-4">
                <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-1">Incremental</h3>
                <div className="text-xs space-y-1 bg-gray-50 rounded p-2 max-w-md">
                  <div className="flex justify-between">
                    <span className="text-gray-500">Granularity</span>
                    <span className="font-medium text-gray-700">{detail.incremental.granularity}</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-500">Partition</span>
                    <span className="font-mono text-gray-700">{detail.incremental.partition_column}</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-500">Event Time</span>
                    <span className="font-mono text-gray-700">{detail.incremental.event_time_column}</span>
                  </div>
                  {detail.incremental.unique_key.length > 0 && (
                    <div className="flex justify-between">
                      <span className="text-gray-500">Unique Key</span>
                      <span className="font-mono text-gray-700">{detail.incremental.unique_key.join(', ')}</span>
                    </div>
                  )}
                </div>
              </div>
            )}

            {detail.columns.length > 0 && (
              <div className="mb-4">
                <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">
                  Columns ({detail.columns.length})
                </h3>
                <ColumnsTable columns={detail.columns} />
              </div>
            )}

            <div>
              <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">Model SQL</h3>
              <SqlViewer sql={detail.sql} removeComments={removeComments} />
            </div>
          </section>
        )}

        {diagnostics && (
          <>
            <PropertiesSection properties={diagnostics.properties} />

            <section className="mb-6">
              <h2 className="text-sm font-semibold text-gray-900 mb-2">Relation Contract</h2>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                <RelationContractBlock title={model} contract={diagnostics.contract} />
                {diagnostics.inbound_edges.map((edge) => (
                  <RelationContractBlock key={edge.name} title={`${edge.name} (${edge.provider})`} contract={edge.contract} />
                ))}
              </div>
            </section>

            <section className="mb-6">
              <h2 className="text-sm font-semibold text-gray-900 mb-2">Maintenance Plan</h2>
              {diagnostics.cells.length === 0 && (
                <p className="text-xs text-gray-400">No maintenance cells for this model.</p>
              )}
              {diagnostics.cells.map((cell, i) => (
                <TechniquePreviewBlock key={`${cell.group}-${i}`} cell={cell} removeComments={removeComments} />
              ))}
            </section>
          </>
        )}
      </div>
    </div>
  )
}
