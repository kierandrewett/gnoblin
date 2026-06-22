/* Validates the rounded-rectangle signed-distance field that gnoblin-shadow.cpp
 * and gnoblin-rounded.cpp compute in GLSL (the shape behind the soft shadow and
 * the corner mask). Same math, on the CPU, asserted at known points. */
#include <math.h>
#include <stdio.h>

static float rrect_sdf(float px, float py, float w, float h, float radius) {
    float cx = w * 0.5f, cy = h * 0.5f;
    float qx = fabsf(px - cx) - (w * 0.5f - radius);
    float qy = fabsf(py - cy) - (h * 0.5f - radius);
    float outside = sqrtf(fmaxf(qx, 0.f) * fmaxf(qx, 0.f) + fmaxf(qy, 0.f) * fmaxf(qy, 0.f));
    float inside = fminf(fmaxf(qx, qy), 0.f);
    return inside + outside - radius;
}
static int near(float a, float b) { return fabsf(a - b) < 0.5f; }

int main(void) {
    float w = 200, h = 100, r = 20;
    int ok = 1;
    float center = rrect_sdf(100, 50, w, h, r);   /* deep inside -> negative   */
    float on_edge = rrect_sdf(200, 50, w, h, r);  /* right edge      -> ~0      */
    float outside = rrect_sdf(210, 50, w, h, r);  /* 10px past edge  -> ~+10    */
    float corner  = rrect_sdf(250, 130, w, h, r); /* past corner     -> positive*/

    if (!(center < 0))    { printf("FAIL center=%f (want <0)\n", center); ok = 0; }
    if (!near(on_edge,0)) { printf("FAIL edge=%f (want ~0)\n", on_edge); ok = 0; }
    if (!near(outside,10)){ printf("FAIL outside=%f (want ~10)\n", outside); ok = 0; }
    if (!(corner > 0))    { printf("FAIL corner=%f (want >0)\n", corner); ok = 0; }

    if (ok) { printf("PASS: rounded-rect SDF geometry (shadow + corner mask) correct\n"); return 0; }
    return 1;
}
