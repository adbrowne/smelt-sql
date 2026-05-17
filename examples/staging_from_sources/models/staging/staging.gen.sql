---
generates: models
---
-- Staging generator: one model per raw source.
-- Demonstrates smelt.sources.with_tag inside a generator body (allowed;
-- sources are loader-time, not model-shape — unlike smelt.models.* which
-- is forbidden inside generator bodies).
[
  ModelDef {
    name: 'orders',
    description: 'Staging layer for raw orders',
    body: SELECT id, user_id, amount, created_at FROM smelt.sources.raw.orders
  },
  ModelDef {
    name: 'users',
    description: 'Staging layer for raw users',
    body: SELECT id, email, created_at FROM smelt.sources.raw.users
  },
  ModelDef {
    name: 'products',
    description: 'Staging layer for raw products',
    body: SELECT id, name, price FROM smelt.sources.raw.products
  }
]
