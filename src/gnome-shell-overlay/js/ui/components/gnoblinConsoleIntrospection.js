import GIRepository from "gi://GIRepository";

const TYPE_NAMES = new Map([
    [GIRepository.TypeTag.VOID, "void"],
    [GIRepository.TypeTag.BOOLEAN, "boolean"],
    [GIRepository.TypeTag.INT8, "int8"],
    [GIRepository.TypeTag.UINT8, "uint8"],
    [GIRepository.TypeTag.INT16, "int16"],
    [GIRepository.TypeTag.UINT16, "uint16"],
    [GIRepository.TypeTag.INT32, "int32"],
    [GIRepository.TypeTag.UINT32, "uint32"],
    [GIRepository.TypeTag.INT64, "int64"],
    [GIRepository.TypeTag.UINT64, "uint64"],
    [GIRepository.TypeTag.FLOAT, "float"],
    [GIRepository.TypeTag.DOUBLE, "double"],
    [GIRepository.TypeTag.GTYPE, "GType"],
    [GIRepository.TypeTag.UTF8, "string"],
    [GIRepository.TypeTag.FILENAME, "filename"],
    [GIRepository.TypeTag.ERROR, "GLib.Error"],
    [GIRepository.TypeTag.UNICHAR, "unichar"],
]);

function safe(call, fallback = null) {
    try {
        return call();
    } catch {
        return fallback;
    }
}

function typeName(type) {
    if (!type) return "unknown";
    const tag = safe(() => type.get_tag());
    if (tag === GIRepository.TypeTag.INTERFACE) {
        const iface = safe(() => type.get_interface());
        if (iface) return `${safe(() => iface.get_namespace(), "GObject")}.${safe(() => iface.get_name(), "Object")}`;
        return "object";
    }
    if (tag === GIRepository.TypeTag.ARRAY) {
        const element = safe(() => type.get_param_type(0));
        return `${typeName(element)}[]`;
    }
    if (tag === GIRepository.TypeTag.GLIST || tag === GIRepository.TypeTag.GSLIST) {
        const element = safe(() => type.get_param_type(0));
        return `List<${typeName(element)}>`;
    }
    if (tag === GIRepository.TypeTag.GHASH) {
        const key = safe(() => type.get_param_type(0));
        const value = safe(() => type.get_param_type(1));
        return `Map<${typeName(key)}, ${typeName(value)}>`;
    }
    return TYPE_NAMES.get(tag) ?? "unknown";
}

function infoKind(info) {
    const name = info?.constructor?.name ?? "";
    if (name.includes("FunctionInfo")) return "function";
    if (name.includes("VFuncInfo")) return "method";
    if (name.includes("CallbackInfo")) return "callback";
    if (name.includes("PropertyInfo")) return "property";
    if (name.includes("SignalInfo")) return "signal";
    if (name.includes("ConstantInfo")) return "constant";
    if (name.includes("FieldInfo")) return "field";
    if (name.includes("EnumInfo") || name.includes("FlagsInfo")) return "enum";
    if (name.includes("ObjectInfo") || name.includes("InterfaceInfo") || name.includes("StructInfo")) return "type";
    return typeof info?.get_n_args === "function" ? "function" : "value";
}

function callableSignature(info) {
    if (!info || typeof info.get_n_args !== "function") return null;
    const name = safe(() => info.get_name(), "call");
    const parameters = [];
    for (let index = 0; index < safe(() => info.get_n_args(), 0); index++) {
        const argument = safe(() => info.get_arg(index));
        if (!argument || safe(() => argument.is_skip(), false)) continue;
        const argumentName = safe(() => argument.get_name(), `arg${index}`);
        const optional = safe(() => argument.is_optional(), false);
        const direction = safe(() => argument.get_direction(), GIRepository.Direction.IN);
        const directionName =
            direction === GIRepository.Direction.OUT
                ? "out "
                : direction === GIRepository.Direction.INOUT
                  ? "inout "
                  : "";
        const type = typeName(safe(() => argument.get_type_info()));
        parameters.push({
            label: `${directionName}${argumentName}${optional ? "?" : ""}: ${type}`,
            documentation: `${type}${optional ? " · optional" : ""}${directionName ? ` · ${directionName.trim()}` : ""}`,
        });
    }
    const returnType = typeName(safe(() => info.get_return_type()));
    return {
        label: `${name}(${parameters.map((parameter) => parameter.label).join(", ")}) -> ${returnType}`,
        documentation: `GObject Introspection signature for ${safe(() => info.get_namespace(), "")}.${name}.`,
        parameters,
    };
}

function findNamed(info, name) {
    if (!info) return null;
    if (name === "prototype") return info;
    for (const [countName, itemName] of [
        ["get_n_methods", "get_method"],
        ["get_n_properties", "get_property"],
        ["get_n_signals", "get_signal"],
        ["get_n_constants", "get_constant"],
        ["get_n_fields", "get_field"],
        ["get_n_vfuncs", "get_vfunc"],
    ]) {
        const count = safe(() => (typeof info[countName] === "function" ? info[countName]() : 0), 0);
        for (let index = 0; index < count; index++) {
            const child = safe(() => info[itemName](index));
            if (safe(() => child?.get_name()) === name) return child;
        }
    }
    const parent = safe(() => info.get_parent());
    return parent ? findNamed(parent, name) : null;
}

export function createGjsIntrospector(namespaces = {}) {
    const repository = new GIRepository.Repository();
    const loaded = new Set();

    function ensure(namespace) {
        if (!Object.hasOwn(namespaces, namespace)) return false;
        if (loaded.has(namespace)) return true;
        try {
            repository.require(namespace, null, 0);
            loaded.add(namespace);
            return true;
        } catch {
            return false;
        }
    }

    function infoForPath(path) {
        const parts = String(path).split(".").filter(Boolean);
        const namespace = parts.shift();
        const first = parts.shift();
        if (!namespace || !first || !ensure(namespace)) return null;
        let info = safe(() => repository.find_by_name(namespace, first));
        if (!info) return null;
        for (const part of parts) {
            info = findNamed(info, part);
            if (!info) return null;
        }
        return info;
    }

    function describe(path) {
        const info = infoForPath(path);
        if (!info) return null;
        const kind = infoKind(info);
        const signature = callableSignature(info);
        if (signature) return { path, kind, ...signature };
        if (kind === "property" || kind === "field") {
            return {
                path,
                kind,
                detail: `${kind}: ${typeName(safe(() => info.get_type_info()))}`,
                documentation: `GObject Introspection ${kind} ${path}.`,
            };
        }
        const name = safe(() => info.get_name(), path);
        return {
            path,
            kind,
            detail: `${kind}: ${safe(() => info.get_namespace(), "")}.${name}`,
            documentation: `GObject Introspection ${kind} ${safe(() => info.get_namespace(), "")}.${name}.`,
        };
    }

    return { describe, signature: (path) => callableSignature(infoForPath(path)) };
}
