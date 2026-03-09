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


def model(func):
    _registered_models[func.__name__] = func
    return func
