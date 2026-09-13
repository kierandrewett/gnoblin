/* SPDX-License-Identifier: MIT */
#include "paint.h"
#include <QGuiApplication>
#include <QImage>
#include <QPainter>
static QGuiApplication* application;
extern "C" void frame_paint_init(int* argc, char*** argv) {
    // Rendering only: Qt must not acquire an xdg-toplevel role of its own.
    qputenv("QT_QPA_PLATFORM", "offscreen");
    application = new QGuiApplication(*argc, *argv);
}
extern "C" void frame_paint(void* pixels, const FramePaint* m) {
    QImage image(static_cast<uchar*>(pixels), m->width, m->height, m->width * 4,
                 QImage::Format_ARGB32_Premultiplied);
    QPainter painter(&image);
    painter.fillRect(image.rect(), QColor::fromRgba(m->color));
    painter.setPen(Qt::white);
    painter.setFont(QFont("Sans", 10));
    painter.drawText(
        QRect(m->left + 10, 0, qMax(0, m->width - m->left - m->right - 3 * m->top - 20), m->top),
        Qt::AlignVCenter, QString::fromUtf8(m->title));
    for (int i = 0; i < 3; i++)
        painter.drawText(QRect(m->width - m->right - (i + 1) * m->top, 0, m->top, m->top),
                         Qt::AlignCenter, QString::fromUtf8((const char*[]){"×", "□", "−"}[i]));
}
