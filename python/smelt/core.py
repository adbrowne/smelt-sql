_registered_models = {}


class ModelInfo:
    def __init__(self, name, tags=None, directory=None):
        self.name = name
        self.tags = tags or []
        self.directory = directory


class ProjectContext:
    def __init__(self, models_data):
        self._models = [ModelInfo(**m) for m in models_data]
        self._queries = []

    def find_models(self, tag=None, directory=None):
        self._queries.append({"kind": "find_models", "tag": tag, "directory": directory})
        result = self._models
        if tag:
            result = [m for m in result if tag in m.tags]
        if directory:
            result = [m for m in result if m.directory == directory]
        return result


def model(func=None):
    # Support both the bare form `@model` and the called form `@model()`.
    # When used as `@model()`, `func` is None and we return the registrar so
    # Python applies it to the decorated function on the next call.
    def register(f):
        _registered_models[f.__name__] = f
        return f

    if func is None:
        return register
    return register(func)
