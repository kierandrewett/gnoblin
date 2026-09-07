/* Validate user effects on the compositor's GL driver before replacing an effect. */
#include "config.h"
#include "shell-gnoblin-shader.h"
#include <epoxy/gl.h>

static gboolean
compile_shader (GLuint shader, const char *source, GError **error)
{
  GLint status;
  char log[4096] = { 0 };

  glShaderSource (shader, 1, &source, NULL);
  glCompileShader (shader);
  glGetShaderiv (shader, GL_COMPILE_STATUS, &status);
  if (status)
    return TRUE;
  glGetShaderInfoLog (shader, sizeof log, NULL, log);
  g_set_error (error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
               "Shader compile failed: %s", log);
  return FALSE;
}

/**
 * shell_gnoblin_validate_shader:
 * @source: fragment source using the Cogl shader inputs
 * @error: return location for compiler diagnostics
 *
 * Returns: whether the shader compiles and links on the current GL driver
 */
gboolean
shell_gnoblin_validate_shader (const char *source, GError **error)
{
  GLuint vertex, fragment, program = 0;
  GLint status = FALSE;
  char log[4096] = { 0 };
  const char *version = epoxy_is_desktop_gl () ? "#version 120\n" :
                                                "#version 100\nprecision mediump float;\n";
  g_autofree char *vertex_source = g_strconcat (version,
    "attribute vec4 position;\n"
    "varying vec4 cogl_tex_coord_in[1]; varying vec4 cogl_color_in;\n"
    "void main() { gl_Position=position; cogl_tex_coord_in[0]=position; cogl_color_in=vec4(1.0); }", NULL);
  g_autofree char *fragment_source = g_strconcat (version,
    "varying vec4 cogl_tex_coord_in[1]; varying vec4 cogl_color_in;\n"
    "#define cogl_color_out gl_FragColor\n", source, NULL);

  vertex = glCreateShader (GL_VERTEX_SHADER);
  fragment = glCreateShader (GL_FRAGMENT_SHADER);
  if (!compile_shader (vertex, vertex_source, error) ||
      !compile_shader (fragment, fragment_source, error))
    goto out;
  program = glCreateProgram ();
  glAttachShader (program, vertex);
  glAttachShader (program, fragment);
  glLinkProgram (program);
  glGetProgramiv (program, GL_LINK_STATUS, &status);
  if (!status)
    {
      glGetProgramInfoLog (program, sizeof log, NULL, log);
      g_set_error (error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                   "Shader link failed: %s", log);
    }
out:
  if (program) glDeleteProgram (program);
  glDeleteShader (vertex);
  glDeleteShader (fragment);
  return status;
}
