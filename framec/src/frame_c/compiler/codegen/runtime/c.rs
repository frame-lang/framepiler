//! C-language runtime emission.
//!
//! `generate_c_compartment_types` is the entry point used by the
//! C backend; it delegates to `generate_c_runtime_types` which
//! emits the bulk of the C runtime infrastructure: FrameDict
//! hash map, FrameVec dynamic array, FrameEvent / FrameContext
//! structs, and Compartment types — all prefixed by the system
//! name so multiple systems can coexist in one translation
//! unit. Pure text emission; no per-system shape variation
//! beyond the name prefix.

use crate::frame_c::compiler::frame_ast::SystemAst;

/// Generate C runtime types (public wrapper)
///
/// Generates the standard Frame runtime infrastructure for C:
/// - FrameDict hash map implementation
/// - FrameVec dynamic array implementation
/// - FrameEvent struct
/// - FrameContext struct
/// - Compartment struct
/// All prefixed with the system name (e.g., Minimal_FrameDict)
pub fn generate_c_compartment_types(system: &SystemAst) -> String {
    generate_c_runtime_types(system)
}

/// Generate C runtime types (internal implementation)
fn generate_c_runtime_types(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();

    // Standard includes
    code.push_str("#include <stdlib.h>\n");
    code.push_str("#include <string.h>\n");
    code.push_str("#include <stdio.h>\n");
    code.push_str("#include <stdbool.h>\n");
    code.push_str("#include <stdint.h>\n\n");

    // ============================================================================
    // FrameDict - String-keyed hash map
    // ============================================================================
    code.push_str(&format!(
        "// ============================================================================\n"
    ));
    code.push_str(&format!("// {}_FrameDict - String-keyed dictionary\n", sys));
    code.push_str(&format!(
        "// ============================================================================\n\n"
    ));

    code.push_str(&format!("typedef struct {}_FrameDictEntry {{\n", sys));
    code.push_str("    char* key;\n");
    code.push_str("    void* value;\n");
    code.push_str(&format!("    struct {}_FrameDictEntry* next;\n", sys));
    code.push_str(&format!("}} {}_FrameDictEntry;\n\n", sys));

    code.push_str(&format!("typedef struct {{\n"));
    code.push_str(&format!("    {}_FrameDictEntry** buckets;\n", sys));
    code.push_str("    int bucket_count;\n");
    code.push_str("    int size;\n");
    code.push_str(&format!("}} {}_FrameDict;\n\n", sys));

    // Hash function
    code.push_str(&format!(
        "static unsigned int {}_hash_string(const char* str) {{\n",
        sys
    ));
    code.push_str("    unsigned int hash = 5381;\n");
    code.push_str("    int c;\n");
    code.push_str("    while ((c = *str++)) {\n");
    code.push_str("        hash = ((hash << 5) + hash) + c;\n");
    code.push_str("    }\n");
    code.push_str("    return hash;\n");
    code.push_str("}\n\n");

    // FrameDict_new
    code.push_str(&format!(
        "static {}_FrameDict* {}_FrameDict_new(void) {{\n",
        sys, sys
    ));
    code.push_str(&format!(
        "    {}_FrameDict* d = malloc(sizeof({}_FrameDict));\n",
        sys, sys
    ));
    code.push_str("    d->bucket_count = 16;\n");
    code.push_str(&format!(
        "    d->buckets = calloc(d->bucket_count, sizeof({}_FrameDictEntry*));\n",
        sys
    ));
    code.push_str("    d->size = 0;\n");
    code.push_str("    return d;\n");
    code.push_str("}\n\n");

    // FrameDict_set
    code.push_str(&format!(
        "static void {}_FrameDict_set({}_FrameDict* d, const char* key, void* value) {{\n",
        sys, sys
    ));
    code.push_str(&format!(
        "    unsigned int idx = {}_hash_string(key) % d->bucket_count;\n",
        sys
    ));
    code.push_str(&format!(
        "    {}_FrameDictEntry* entry = d->buckets[idx];\n",
        sys
    ));
    code.push_str("    while (entry) {\n");
    code.push_str("        if (strcmp(entry->key, key) == 0) {\n");
    code.push_str("            entry->value = value;\n");
    code.push_str("            return;\n");
    code.push_str("        }\n");
    code.push_str("        entry = entry->next;\n");
    code.push_str("    }\n");
    code.push_str(&format!(
        "    {}_FrameDictEntry* new_entry = malloc(sizeof({}_FrameDictEntry));\n",
        sys, sys
    ));
    code.push_str("    new_entry->key = strdup(key);\n");
    code.push_str("    new_entry->value = value;\n");
    code.push_str("    new_entry->next = d->buckets[idx];\n");
    code.push_str("    d->buckets[idx] = new_entry;\n");
    code.push_str("    d->size++;\n");
    code.push_str("}\n\n");

    // FrameDict_get
    code.push_str(&format!(
        "static void* {}_FrameDict_get({}_FrameDict* d, const char* key) {{\n",
        sys, sys
    ));
    code.push_str(&format!(
        "    unsigned int idx = {}_hash_string(key) % d->bucket_count;\n",
        sys
    ));
    code.push_str(&format!(
        "    {}_FrameDictEntry* entry = d->buckets[idx];\n",
        sys
    ));
    code.push_str("    while (entry) {\n");
    code.push_str("        if (strcmp(entry->key, key) == 0) {\n");
    code.push_str("            return entry->value;\n");
    code.push_str("        }\n");
    code.push_str("        entry = entry->next;\n");
    code.push_str("    }\n");
    code.push_str("    return NULL;\n");
    code.push_str("}\n\n");

    // FrameDict_has - check if key exists
    code.push_str(&format!(
        "static int {}_FrameDict_has({}_FrameDict* d, const char* key) {{\n",
        sys, sys
    ));
    code.push_str(&format!(
        "    unsigned int idx = {}_hash_string(key) % d->bucket_count;\n",
        sys
    ));
    code.push_str(&format!(
        "    {}_FrameDictEntry* entry = d->buckets[idx];\n",
        sys
    ));
    code.push_str("    while (entry) {\n");
    code.push_str("        if (strcmp(entry->key, key) == 0) {\n");
    code.push_str("            return 1;\n");
    code.push_str("        }\n");
    code.push_str("        entry = entry->next;\n");
    code.push_str("    }\n");
    code.push_str("    return 0;\n");
    code.push_str("}\n\n");

    // FrameDict_copy
    code.push_str(&format!(
        "static {}_FrameDict* {}_FrameDict_copy({}_FrameDict* src) {{\n",
        sys, sys, sys
    ));
    code.push_str(&format!(
        "    {}_FrameDict* dst = {}_FrameDict_new();\n",
        sys, sys
    ));
    code.push_str("    for (int i = 0; i < src->bucket_count; i++) {\n");
    code.push_str(&format!(
        "        {}_FrameDictEntry* entry = src->buckets[i];\n",
        sys
    ));
    code.push_str("        while (entry) {\n");
    code.push_str(&format!(
        "            {}_FrameDict_set(dst, entry->key, entry->value);\n",
        sys
    ));
    code.push_str("            entry = entry->next;\n");
    code.push_str("        }\n");
    code.push_str("    }\n");
    code.push_str("    return dst;\n");
    code.push_str("}\n\n");

    // FrameDict_destroy
    code.push_str(&format!(
        "static void {}_FrameDict_destroy({}_FrameDict* d) {{\n",
        sys, sys
    ));
    code.push_str("    for (int i = 0; i < d->bucket_count; i++) {\n");
    code.push_str(&format!(
        "        {}_FrameDictEntry* entry = d->buckets[i];\n",
        sys
    ));
    code.push_str("        while (entry) {\n");
    code.push_str(&format!(
        "            {}_FrameDictEntry* next = entry->next;\n",
        sys
    ));
    code.push_str("            free(entry->key);\n");
    code.push_str("            free(entry);\n");
    code.push_str("            entry = next;\n");
    code.push_str("        }\n");
    code.push_str("    }\n");
    code.push_str("    free(d->buckets);\n");
    code.push_str("    free(d);\n");
    code.push_str("}\n\n");

    // ============================================================================
    // FrameVec - Dynamic array
    // ============================================================================
    code.push_str(&format!(
        "// ============================================================================\n"
    ));
    code.push_str(&format!("// {}_FrameVec - Dynamic array\n", sys));
    code.push_str(&format!(
        "// ============================================================================\n\n"
    ));

    code.push_str(&format!("typedef struct {{\n"));
    code.push_str("    void** items;\n");
    code.push_str("    int size;\n");
    code.push_str("    int capacity;\n");
    code.push_str(&format!("}} {}_FrameVec;\n\n", sys));

    // FrameVec_new
    code.push_str(&format!(
        "static {}_FrameVec* {}_FrameVec_new(void) {{\n",
        sys, sys
    ));
    code.push_str(&format!(
        "    {}_FrameVec* v = malloc(sizeof({}_FrameVec));\n",
        sys, sys
    ));
    code.push_str("    v->capacity = 8;\n");
    code.push_str("    v->size = 0;\n");
    code.push_str("    v->items = malloc(sizeof(void*) * v->capacity);\n");
    code.push_str("    return v;\n");
    code.push_str("}\n\n");

    // FrameVec_push
    code.push_str(&format!(
        "static void {}_FrameVec_push({}_FrameVec* v, void* item) {{\n",
        sys, sys
    ));
    code.push_str("    if (v->size >= v->capacity) {\n");
    code.push_str("        v->capacity *= 2;\n");
    code.push_str("        v->items = realloc(v->items, sizeof(void*) * v->capacity);\n");
    code.push_str("    }\n");
    code.push_str("    v->items[v->size++] = item;\n");
    code.push_str("}\n\n");

    // FrameVec_pop
    code.push_str(&format!(
        "static void* {}_FrameVec_pop({}_FrameVec* v) {{\n",
        sys, sys
    ));
    code.push_str("    if (v->size == 0) return NULL;\n");
    code.push_str("    return v->items[--v->size];\n");
    code.push_str("}\n\n");

    // FrameVec_last
    code.push_str(&format!(
        "static void* {}_FrameVec_last({}_FrameVec* v) {{\n",
        sys, sys
    ));
    code.push_str("    if (v->size == 0) return NULL;\n");
    code.push_str("    return v->items[v->size - 1];\n");
    code.push_str("}\n\n");

    // FrameVec_get (indexed access)
    code.push_str(&format!(
        "static void* {}_FrameVec_get({}_FrameVec* v, int index) {{\n",
        sys, sys
    ));
    code.push_str("    if (index < 0 || index >= v->size) return NULL;\n");
    code.push_str("    return v->items[index];\n");
    code.push_str("}\n\n");

    // FrameVec_size
    code.push_str(&format!(
        "static int {}_FrameVec_size({}_FrameVec* v) {{\n",
        sys, sys
    ));
    code.push_str("    return v->size;\n");
    code.push_str("}\n\n");

    // FrameVec_destroy
    code.push_str(&format!(
        "static void {}_FrameVec_destroy({}_FrameVec* v) {{\n",
        sys, sys
    ));
    code.push_str("    if (!v) return;\n");
    code.push_str("    free(v->items);\n");
    code.push_str("    free(v);\n");
    code.push_str("}\n\n");

    // FrameVec_copy — shallow copy (items are void*; caller owns pointees).
    // Used by Compartment_copy when snapshotting args for push$.
    code.push_str(&format!(
        "static {}_FrameVec* {}_FrameVec_copy({}_FrameVec* src) {{\n",
        sys, sys, sys
    ));
    code.push_str("    if (!src) return NULL;\n");
    code.push_str(&format!(
        "    {}_FrameVec* v = {}_FrameVec_new();\n",
        sys, sys
    ));
    code.push_str("    for (int i = 0; i < src->size; i++) {\n");
    code.push_str(&format!(
        "        {}_FrameVec_push(v, src->items[i]);\n",
        sys
    ));
    code.push_str("    }\n");
    code.push_str("    return v;\n");
    code.push_str("}\n\n");

    // ============================================================================
    // Double-return marshalling helpers
    // ============================================================================
    // `_return` is a `void*` slot. Casting a `double` through `(intptr_t)`
    // truncates the fractional part, and casting a `void*` back to `double`
    // is illegal C. Bit-pun through `memcpy` — legal and round-trips
    // cleanly on every 64-bit target (both `double` and `void*` are 8
    // bytes). The C backend emits calls to these wherever a handler's
    // return type is `float` / `double`.
    code.push_str(&format!(
        "// ============================================================================\n"
    ));
    code.push_str(&format!(
        "// {}_pack_double / {}_unpack_double — bit-pun doubles through void*\n",
        sys, sys
    ));
    code.push_str(&format!(
        "// ============================================================================\n\n"
    ));
    code.push_str(&format!(
        "static inline void* {}_pack_double(double v) {{\n",
        sys
    ));
    code.push_str("    void* p = 0;\n");
    code.push_str("    memcpy(&p, &v, sizeof(double));\n");
    code.push_str("    return p;\n");
    code.push_str("}\n\n");
    code.push_str(&format!(
        "static inline double {}_unpack_double(void* p) {{\n",
        sys
    ));
    code.push_str("    double d;\n");
    code.push_str("    memcpy(&d, &p, sizeof(double));\n");
    code.push_str("    return d;\n");
    code.push_str("}\n\n");

    // ============================================================================
    // Persist dispatcher — type-ignorant codegen
    // ============================================================================
    // framec mangles each field's declared C type to a symbol suffix
    // and emits `<sys>_persist_pack_<suffix>(value)` /
    // `<sys>_persist_unpack_<suffix>(json)` calls. The runtime defines
    // the symbols for blessed types (int / str / bool / double / list /
    // dict). User-defined types are extended by defining additional
    // symbols of the same shape.
    //
    // This isolates type knowledge to the runtime+user — framec only
    // mangles strings to identifiers.
    //
    // Only emit when @@persist is on — the helpers reference cJSON
    // types, which require the user's `#include <cjson/cJSON.h>` in
    // the prolog. Non-persist systems shouldn't pull cJSON.
    if system.persist_attr.is_some() {
        code.push_str(&format!(
        "// ============================================================================\n// {}_persist_pack_*  /  {}_persist_unpack_*  — persist dispatcher\n// ============================================================================\n\n",
        sys, sys
    ));

        // Forward decls so the recursive list/dict packers compile.
        code.push_str(&format!("static cJSON* {sys}_persist_pack_int(void* v);\n"));
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_double(void* v);\n"
        ));
        code.push_str(&format!("static cJSON* {sys}_persist_pack_str(void* v);\n"));
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_bool(void* v);\n"
        ));
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_list(void* v);\n"
        ));
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_dict(void* v);\n"
        ));
        code.push_str(&format!(
            "static void* {sys}_persist_unpack_int(cJSON* j);\n"
        ));
        code.push_str(&format!(
            "static void* {sys}_persist_unpack_double(cJSON* j);\n"
        ));
        code.push_str(&format!(
            "static void* {sys}_persist_unpack_str(cJSON* j);\n"
        ));
        code.push_str(&format!(
            "static void* {sys}_persist_unpack_bool(cJSON* j);\n"
        ));
        code.push_str(&format!(
            "static void* {sys}_persist_unpack_list(cJSON* j);\n"
        ));
        code.push_str(&format!(
            "static void* {sys}_persist_unpack_dict(cJSON* j);\n\n"
        ));

        // int — `(intptr_t)int` round-trip.
        code.push_str(&format!(
        "static cJSON* {sys}_persist_pack_int(void* v) {{ return cJSON_CreateNumber((double)(intptr_t)v); }}\n"
    ));
        code.push_str(&format!(
        "static void* {sys}_persist_unpack_int(cJSON* j) {{ return (void*)(intptr_t)(int)(j ? j->valuedouble : 0); }}\n\n"
    ));

        // double — bit-pun via the existing helpers above.
        code.push_str(&format!(
        "static cJSON* {sys}_persist_pack_double(void* v) {{ return cJSON_CreateNumber({sys}_unpack_double(v)); }}\n"
    ));
        code.push_str(&format!(
        "static void* {sys}_persist_unpack_double(cJSON* j) {{ return {sys}_pack_double(j ? j->valuedouble : 0.0); }}\n\n"
    ));

        // str — char* round-trip. Note: the void* slot stores the pointer
        // directly; ownership semantics are the user's responsibility (same
        // as the current C codegen).
        code.push_str(&format!(
        "static cJSON* {sys}_persist_pack_str(void* v) {{ return cJSON_CreateString(v ? (const char*)v : \"\"); }}\n"
    ));
        code.push_str(&format!(
        "static void* {sys}_persist_unpack_str(cJSON* j) {{ const char* s = (j && j->valuestring) ? j->valuestring : \"\"; return (void*)strdup(s); }}\n\n"
    ));

        // bool — JSON true/false.
        code.push_str(&format!(
        "static cJSON* {sys}_persist_pack_bool(void* v) {{ return cJSON_CreateBool((intptr_t)v != 0); }}\n"
    ));
        code.push_str(&format!(
        "static void* {sys}_persist_unpack_bool(cJSON* j) {{ return (void*)(intptr_t)(j && cJSON_IsTrue(j) ? 1 : 0); }}\n\n"
    ));

        // list — recurse into FrameVec elements as int. User extends by
        // defining their own pack_<their_list_type> for non-int elements.
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_list(void* v) {{\n\
         \x20   {sys}_FrameVec* vec = ({sys}_FrameVec*)v;\n\
         \x20   cJSON* arr = cJSON_CreateArray();\n\
         \x20   if (vec) {{\n\
         \x20       for (int __i = 0; __i < vec->size; __i++) {{\n\
         \x20           cJSON_AddItemToArray(arr, {sys}_persist_pack_int(vec->items[__i]));\n\
         \x20       }}\n\
         \x20   }}\n\
         \x20   return arr;\n\
         }}\n"
        ));
        code.push_str(&format!(
            "static void* {sys}_persist_unpack_list(cJSON* j) {{\n\
         \x20   if (!cJSON_IsArray(j)) return NULL;\n\
         \x20   {sys}_FrameVec* vec = {sys}_FrameVec_new();\n\
         \x20   cJSON* __child;\n\
         \x20   cJSON_ArrayForEach(__child, j) {{\n\
         \x20       {sys}_FrameVec_push(vec, {sys}_persist_unpack_int(__child));\n\
         \x20   }}\n\
         \x20   return vec;\n\
         }}\n\n"
        ));

        // dict — recurse into FrameDict entries as int values (string keys
        // — JSON's only key type).
        code.push_str(&format!(
        "static cJSON* {sys}_persist_pack_dict(void* v) {{\n\
         \x20   {sys}_FrameDict* d = ({sys}_FrameDict*)v;\n\
         \x20   cJSON* obj = cJSON_CreateObject();\n\
         \x20   if (d) {{\n\
         \x20       for (int __i = 0; __i < d->bucket_count; __i++) {{\n\
         \x20           {sys}_FrameDictEntry* __e = d->buckets[__i];\n\
         \x20           while (__e) {{\n\
         \x20               cJSON_AddItemToObject(obj, __e->key, {sys}_persist_pack_int(__e->value));\n\
         \x20               __e = __e->next;\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20   }}\n\
         \x20   return obj;\n\
         }}\n"
    ));
        code.push_str(&format!(
            "static void* {sys}_persist_unpack_dict(cJSON* j) {{\n\
         \x20   if (!cJSON_IsObject(j)) return NULL;\n\
         \x20   {sys}_FrameDict* d = {sys}_FrameDict_new();\n\
         \x20   cJSON* __child;\n\
         \x20   cJSON_ArrayForEach(__child, j) {{\n\
         \x20       {sys}_FrameDict_set(d, __child->string, {sys}_persist_unpack_int(__child));\n\
         \x20   }}\n\
         \x20   return d;\n\
         }}\n\n"
        ));

        // Domain-field variants — take `(void*)&self->x` (a pointer to
        // the statically-typed field) and blind-cast it to the right
        // pointer type inside, so the codegen never has to branch on
        // int-vs-str-vs-… and never has to cast the field at the call
        // site. framec emits
        // `<sys>_persist_pack_field_<mangled>((void*)&self->x)` /
        // `<sys>_persist_unpack_field_<mangled>(json, (void*)&self->x)`;
        // it mangles the declared type to a symbol suffix (see
        // `c_mangle_type` in interface_gen.rs and
        // docs/contributing/type-ignorant-codegen.md). The `void*`
        // (rather than a typed `int*` / `char**` / …) keeps the
        // signature uniform regardless of whether the field is `char*`
        // or `const char*`. Blessed types: int / double / str / bool /
        // list / dict; a domain field of a user-defined type extends the
        // set the same way as the value-form helpers above — define
        // `<sys>_persist_pack_field_<that_type>` / `_unpack_field_<…>`.
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_field_int(void* p) {{ return cJSON_CreateNumber((double)*(int*)p); }}\n"
        ));
        code.push_str(&format!(
            "static void {sys}_persist_unpack_field_int(cJSON* j, void* p) {{ *(int*)p = (int)(j ? j->valuedouble : 0); }}\n"
        ));
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_field_double(void* p) {{ return cJSON_CreateNumber(*(double*)p); }}\n"
        ));
        code.push_str(&format!(
            "static void {sys}_persist_unpack_field_double(cJSON* j, void* p) {{ *(double*)p = j ? j->valuedouble : 0.0; }}\n"
        ));
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_field_bool(void* p) {{ return cJSON_CreateBool(*(bool*)p); }}\n"
        ));
        code.push_str(&format!(
            "static void {sys}_persist_unpack_field_bool(cJSON* j, void* p) {{ *(bool*)p = (bool)(j && cJSON_IsTrue(j)); }}\n"
        ));
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_field_str(void* p) {{ const char* s = *(const char**)p; return cJSON_CreateString(s ? s : \"\"); }}\n"
        ));
        code.push_str(&format!(
            "static void {sys}_persist_unpack_field_str(cJSON* j, void* p) {{ const char* s = (j && j->valuestring) ? j->valuestring : \"\"; *(char**)p = strdup(s); }}\n"
        ));
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_field_list(void* p) {{ return {sys}_persist_pack_list(*({sys}_FrameVec**)p); }}\n"
        ));
        code.push_str(&format!(
            "static void {sys}_persist_unpack_field_list(cJSON* j, void* p) {{ *({sys}_FrameVec**)p = ({sys}_FrameVec*){sys}_persist_unpack_list(j); }}\n"
        ));
        code.push_str(&format!(
            "static cJSON* {sys}_persist_pack_field_dict(void* p) {{ return {sys}_persist_pack_dict(*({sys}_FrameDict**)p); }}\n"
        ));
        code.push_str(&format!(
            "static void {sys}_persist_unpack_field_dict(cJSON* j, void* p) {{ *({sys}_FrameDict**)p = ({sys}_FrameDict*){sys}_persist_unpack_dict(j); }}\n\n"
        ));
    } // end persist_attr guard

    // ============================================================================
    // FrameEvent - Event routing object
    // ============================================================================
    code.push_str(&format!(
        "// ============================================================================\n"
    ));
    code.push_str(&format!("// {}_FrameEvent - Event routing object\n", sys));
    code.push_str(&format!(
        "// ============================================================================\n\n"
    ));

    code.push_str(&format!("typedef struct {{\n"));
    code.push_str("    const char* _message;\n");
    code.push_str(&format!("    {}_FrameVec* _parameters;\n", sys));
    code.push_str("    int _owns_parameters;\n");
    code.push_str(&format!("}} {}_FrameEvent;\n\n", sys));

    // FrameEvent_new — owns_parameters=1 means this event allocated the vec.
    // Parameters are positional: dispatch reads `_parameters->items[N]`.
    code.push_str(&format!("static {}_FrameEvent* {}_FrameEvent_new(const char* message, {}_FrameVec* parameters, int owns_parameters) {{\n", sys, sys, sys));
    code.push_str(&format!(
        "    {}_FrameEvent* e = malloc(sizeof({}_FrameEvent));\n",
        sys, sys
    ));
    code.push_str("    e->_message = message;\n");
    code.push_str("    e->_parameters = parameters;\n");
    code.push_str("    e->_owns_parameters = owns_parameters;\n");
    code.push_str("    return e;\n");
    code.push_str("}\n\n");

    // FrameEvent_destroy — only frees parameters if this event owns them
    code.push_str(&format!(
        "static void {}_FrameEvent_destroy({}_FrameEvent* e) {{\n",
        sys, sys
    ));
    code.push_str(&format!(
        "    if (e->_owns_parameters && e->_parameters) {}_FrameVec_destroy(e->_parameters);\n",
        sys
    ));
    code.push_str("    free(e);\n");
    code.push_str("}\n\n");

    // ============================================================================
    // FrameContext - Interface call context
    // ============================================================================
    code.push_str(&format!(
        "// ============================================================================\n"
    ));
    code.push_str(&format!(
        "// {}_FrameContext - Interface call context\n",
        sys
    ));
    code.push_str(&format!(
        "// ============================================================================\n\n"
    ));

    code.push_str(&format!("typedef struct {{\n"));
    code.push_str(&format!("    {}_FrameEvent* event;\n", sys));
    code.push_str("    void* _return;\n");
    code.push_str(&format!("    {}_FrameDict* _data;\n", sys));
    code.push_str("    int _transitioned;\n");
    code.push_str(&format!("}} {}_FrameContext;\n\n", sys));

    // FrameContext_new
    code.push_str(&format!("static {}_FrameContext* {}_FrameContext_new({}_FrameEvent* event, void* default_return) {{\n", sys, sys, sys));
    code.push_str(&format!(
        "    {}_FrameContext* ctx = malloc(sizeof({}_FrameContext));\n",
        sys, sys
    ));
    code.push_str("    ctx->event = event;\n");
    code.push_str("    ctx->_return = default_return;\n");
    code.push_str(&format!("    ctx->_data = {}_FrameDict_new();\n", sys));
    code.push_str("    ctx->_transitioned = 0;\n");
    code.push_str("    return ctx;\n");
    code.push_str("}\n\n");

    // FrameContext_destroy
    code.push_str(&format!(
        "static void {}_FrameContext_destroy({}_FrameContext* ctx) {{\n",
        sys, sys
    ));
    code.push_str(&format!("    {}_FrameDict_destroy(ctx->_data);\n", sys));
    code.push_str("    free(ctx);\n");
    code.push_str("}\n\n");

    // ============================================================================
    // Compartment - State closure
    // ============================================================================
    code.push_str(&format!(
        "// ============================================================================\n"
    ));
    code.push_str(&format!("// {}_Compartment - State closure\n", sys));
    code.push_str(&format!(
        "// ============================================================================\n\n"
    ));

    // state_args / enter_args / exit_args are POSITIONAL — codegen accesses
    // them as `args->items[N]`, so they're FrameVec not FrameDict.
    // state_vars is KEYED by variable name (`$.varName`), so it stays dict.
    code.push_str(&format!("typedef struct {}_Compartment {{\n", sys));
    code.push_str("    const char* state;\n");
    code.push_str(&format!("    {}_FrameVec* state_args;\n", sys));
    code.push_str(&format!("    {}_FrameDict* state_vars;\n", sys));
    code.push_str(&format!("    {}_FrameVec* enter_args;\n", sys));
    code.push_str(&format!("    {}_FrameVec* exit_args;\n", sys));
    code.push_str(&format!("    {}_FrameEvent* forward_event;\n", sys));
    code.push_str(&format!(
        "    struct {}_Compartment* parent_compartment;\n",
        sys
    ));
    code.push_str("    int _ref_count;\n");
    code.push_str(&format!("}} {}_Compartment;\n\n", sys));

    // Compartment_new
    code.push_str(&format!(
        "static {}_Compartment* {}_Compartment_new(const char* state) {{\n",
        sys, sys
    ));
    code.push_str(&format!(
        "    {}_Compartment* c = malloc(sizeof({}_Compartment));\n",
        sys, sys
    ));
    code.push_str("    c->state = state;\n");
    code.push_str(&format!("    c->state_args = {}_FrameVec_new();\n", sys));
    code.push_str(&format!("    c->state_vars = {}_FrameDict_new();\n", sys));
    code.push_str(&format!("    c->enter_args = {}_FrameVec_new();\n", sys));
    code.push_str(&format!("    c->exit_args = {}_FrameVec_new();\n", sys));
    code.push_str("    c->forward_event = NULL;\n");
    code.push_str("    c->parent_compartment = NULL;\n");
    code.push_str("    c->_ref_count = 1;\n");
    code.push_str("    return c;\n");
    code.push_str("}\n\n");

    // Compartment_ref — increment reference count
    code.push_str(&format!(
        "static {sys}_Compartment* {sys}_Compartment_ref({sys}_Compartment* c) {{\n"
    ));
    code.push_str("    if (c) c->_ref_count++;\n");
    code.push_str("    return c;\n");
    code.push_str("}\n\n");

    // Compartment_unref — decrement reference count, destroy when zero
    code.push_str(&format!(
        "static void {sys}_Compartment_unref({sys}_Compartment* c);\n"
    ));
    code.push_str(&format!(
        "static void {sys}_Compartment_unref({sys}_Compartment* c) {{\n"
    ));
    code.push_str("    if (c == NULL) return;\n");
    code.push_str("    c->_ref_count--;\n");
    code.push_str("    if (c->_ref_count <= 0) {\n");
    code.push_str(&format!(
        "        {sys}_Compartment_unref(c->parent_compartment);\n"
    ));
    code.push_str(&format!("        {sys}_FrameVec_destroy(c->state_args);\n"));
    code.push_str(&format!(
        "        {sys}_FrameDict_destroy(c->state_vars);\n"
    ));
    code.push_str(&format!("        {sys}_FrameVec_destroy(c->enter_args);\n"));
    code.push_str(&format!("        {sys}_FrameVec_destroy(c->exit_args);\n"));
    code.push_str("        free(c);\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // Compartment_copy
    code.push_str(&format!(
        "static {}_Compartment* {}_Compartment_copy({}_Compartment* src) {{\n",
        sys, sys, sys
    ));
    code.push_str(&format!(
        "    {}_Compartment* c = malloc(sizeof({}_Compartment));\n",
        sys, sys
    ));
    code.push_str("    c->state = src->state;\n");
    code.push_str(&format!(
        "    c->state_args = {}_FrameVec_copy(src->state_args);\n",
        sys
    ));
    code.push_str(&format!(
        "    c->state_vars = {}_FrameDict_copy(src->state_vars);\n",
        sys
    ));
    code.push_str(&format!(
        "    c->enter_args = {}_FrameVec_copy(src->enter_args);\n",
        sys
    ));
    code.push_str(&format!(
        "    c->exit_args = {}_FrameVec_copy(src->exit_args);\n",
        sys
    ));
    code.push_str("    c->forward_event = src->forward_event;  // Shallow copy OK\n");
    code.push_str("    c->parent_compartment = src->parent_compartment;\n");
    code.push_str("    return c;\n");
    code.push_str("}\n\n");

    // Compartment_destroy
    code.push_str(&format!(
        "static void {}_Compartment_destroy({}_Compartment* c) {{\n",
        sys, sys
    ));
    code.push_str(&format!("    {}_FrameVec_destroy(c->state_args);\n", sys));
    code.push_str(&format!("    {}_FrameDict_destroy(c->state_vars);\n", sys));
    code.push_str(&format!("    {}_FrameVec_destroy(c->enter_args);\n", sys));
    code.push_str(&format!("    {}_FrameVec_destroy(c->exit_args);\n", sys));
    code.push_str("    free(c);\n");
    code.push_str("}\n\n");

    // Helper macros for context access
    code.push_str(&format!("// Helper macros for context access\n"));
    code.push_str(&format!(
        "#define {}_CTX(self) (({}_FrameContext*){}_FrameVec_last((self)->_context_stack))\n",
        sys, sys, sys
    ));
    code.push_str(&format!(
        "#define {}_PARAM(self, key) {}_FrameDict_get({}_CTX(self)->event->_parameters, key)\n",
        sys, sys, sys
    ));
    code.push_str(&format!(
        "#define {}_RETURN(self) {}_CTX(self)->_return\n",
        sys, sys
    ));
    code.push_str(&format!(
        "#define {}_DATA(self, key) {}_FrameDict_get({}_CTX(self)->_data, key)\n",
        sys, sys, sys
    ));
    code.push_str(&format!(
        "#define {}_DATA_SET(self, key, val) {}_FrameDict_set({}_CTX(self)->_data, key, val)\n\n",
        sys, sys, sys
    ));

    // System destroy function (declared as part of forward declarations, defined later)
    // This will be declared as a forward declaration in the class emission

    code
}
