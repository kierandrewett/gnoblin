/* Parse GNOME wallpaper schedules without loading libgnome-desktop in Shell. */

#include <math.h>
#include <stdlib.h>
#include <string.h>

#include <glib.h>
#include <json-glib/json-glib.h>
#include <libxml/parser.h>
#include <libxml/tree.h>

#include "shell-util.h"

static gboolean named(xmlNode* node, const char* name) {
    return node->type == XML_ELEMENT_NODE && xmlStrEqual(node->name, (const xmlChar*)name);
}

static xmlNode* child(xmlNode* parent, const char* name) {
    for (xmlNode* node = parent->children; node; node = node->next)
        if (named(node, name))
            return node;
    return NULL;
}

static char* content(xmlNode* node) {
    if (!node)
        return NULL;
    xmlChar* value = xmlNodeGetContent(node);
    if (!value)
        return NULL;
    char* result = g_strdup((const char*)value);
    xmlFree(value);
    g_strstrip(result);
    return result;
}

static int integer_child(xmlNode* parent, const char* name, int fallback) {
    g_autofree char* value = content(child(parent, name));
    if (!value || !*value)
        return fallback;
    char* end = NULL;
    long parsed = strtol(value, &end, 10);
    return end && !*end && parsed >= 0 && parsed <= G_MAXINT ? parsed : fallback;
}

static void add_file(JsonBuilder* builder, const char* path, int width, int height) {
    if (!path || !*path)
        return;
    json_builder_begin_object(builder);
    json_builder_set_member_name(builder, "path");
    json_builder_add_string_value(builder, path);
    json_builder_set_member_name(builder, "width");
    json_builder_add_int_value(builder, width);
    json_builder_set_member_name(builder, "height");
    json_builder_add_int_value(builder, height);
    json_builder_end_object(builder);
}

static void add_files(JsonBuilder* builder, xmlNode* node, const char* key) {
    json_builder_set_member_name(builder, key);
    json_builder_begin_array(builder);
    if (node) {
        gboolean has_sizes = FALSE;
        for (xmlNode* size = node->children; size; size = size->next) {
            if (!named(size, "size"))
                continue;
            has_sizes = TRUE;
            xmlChar* width_attr = xmlGetProp(size, (const xmlChar*)"width");
            xmlChar* height_attr = xmlGetProp(size, (const xmlChar*)"height");
            int width = width_attr ? MAX(atoi((const char*)width_attr), 0) : 0;
            int height = height_attr ? MAX(atoi((const char*)height_attr), 0) : 0;
            g_autofree char* path = content(size);
            add_file(builder, path, width, height);
            if (width_attr)
                xmlFree(width_attr);
            if (height_attr)
                xmlFree(height_attr);
        }
        if (!has_sizes) {
            g_autofree char* path = content(node);
            add_file(builder, path, -1, -1);
        }
    }
    json_builder_end_array(builder);
}

/**
 * shell_util_parse_wallpaper_slideshow:
 * @xml: contents of a GNOME wallpaper XML file
 * @error: return location for an error
 *
 * Parses the schedule once, after the caller has read the file asynchronously.
 * The returned JSON contains the local start time and ordered slides. Image
 * selection and current-time calculation stay in the JavaScript caller.
 *
 * Returns: (transfer full): the schedule as JSON
 */
char* shell_util_parse_wallpaper_slideshow(const char* xml, GError** error) {
    g_return_val_if_fail(xml != NULL, NULL);

    xmlDoc* document = xmlReadMemory(xml, strlen(xml), "wallpaper.xml", NULL,
                                     XML_PARSE_NONET | XML_PARSE_NOERROR | XML_PARSE_NOWARNING);
    if (!document) {
        g_set_error_literal(error, G_MARKUP_ERROR, G_MARKUP_ERROR_PARSE, "Invalid wallpaper XML");
        return NULL;
    }

    xmlNode* root = xmlDocGetRootElement(document);
    if (!root || !named(root, "background")) {
        g_set_error_literal(error, G_MARKUP_ERROR, G_MARKUP_ERROR_INVALID_CONTENT,
                            "Wallpaper XML has no background element");
        xmlFreeDoc(document);
        return NULL;
    }

    xmlNode* start = child(root, "starttime");
    int year = start ? integer_child(start, "year", 1970) : 1970;
    int month = start ? integer_child(start, "month", 1) : 1;
    int day = start ? integer_child(start, "day", 1) : 1;
    int hour = start ? integer_child(start, "hour", 0) : 0;
    int minute = start ? integer_child(start, "minute", 0) : 0;
    int second = start ? integer_child(start, "second", 0) : 0;
    g_autoptr(GDateTime) start_time = g_date_time_new_local(year, month, day, hour, minute, second);
    if (!start_time) {
        g_set_error_literal(error, G_MARKUP_ERROR, G_MARKUP_ERROR_INVALID_CONTENT,
                            "Wallpaper XML has an invalid start time");
        xmlFreeDoc(document);
        return NULL;
    }

    g_autoptr(JsonBuilder) builder = json_builder_new();
    json_builder_begin_object(builder);
    json_builder_set_member_name(builder, "startTime");
    json_builder_add_int_value(builder, g_date_time_to_unix(start_time));
    json_builder_set_member_name(builder, "slides");
    json_builder_begin_array(builder);
    guint slides = 0;
    for (xmlNode* node = root->children; node; node = node->next) {
        gboolean fixed = named(node, "static");
        if (!fixed && !named(node, "transition"))
            continue;
        g_autofree char* duration_text = content(child(node, "duration"));
        char* end = NULL;
        double duration = duration_text ? g_ascii_strtod(duration_text, &end) : 0;
        if (!duration_text || end == duration_text || *end || !isfinite(duration) || duration <= 0)
            duration = 0;
        json_builder_begin_object(builder);
        json_builder_set_member_name(builder, "duration");
        json_builder_add_double_value(builder, duration);
        json_builder_set_member_name(builder, "fixed");
        json_builder_add_boolean_value(builder, fixed);
        add_files(builder, child(node, fixed ? "file" : "from"), "from");
        add_files(builder, child(node, "to"), "to");
        json_builder_end_object(builder);
        slides++;
    }
    json_builder_end_array(builder);
    json_builder_end_object(builder);
    xmlFreeDoc(document);

    if (slides == 0) {
        g_set_error_literal(error, G_MARKUP_ERROR, G_MARKUP_ERROR_INVALID_CONTENT,
                            "Wallpaper XML has no slides");
        return NULL;
    }

    g_autoptr(JsonNode) result = json_builder_get_root(builder);
    return json_to_string(result, FALSE);
}
