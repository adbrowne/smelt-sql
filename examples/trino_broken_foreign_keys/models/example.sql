-- Broken fixture: the `dev` target in smelt.yml carries three keys that
-- belong to another backend's shape (`warehouse`, `project`, `dataset`).
-- `Config::load` must refuse the whole file, naming every offending key.
SELECT 1 AS id
