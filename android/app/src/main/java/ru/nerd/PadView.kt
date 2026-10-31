package ru.nerd

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.SurfaceTexture
import android.os.SystemClock
import android.view.MotionEvent
import android.view.Surface
import android.view.TextureView
import android.view.View
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.abs
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.min
import kotlin.math.sin
import kotlin.math.sqrt

private const val TAP_SLOP = 24f
private const val LONG_PRESS_MS = 500L

// Локальная линия живёт чуть дольше, чем нужно кадру ПК, чтобы Krita успела её дорисовать
private const val INK_MARGIN_MS = 80L
private const val INK_MAX_AGE_MS = 3000L

enum class Mode(val title: String) { DRAW("Рисовать"), MOUSE("Мышь") }

// Точка локальной линии в координатах монитора; start = начало нового штриха
private class InkPoint(val x: Float, val y: Float, val t: Long, var start: Boolean)

class PadView(context: Context, private val sender: Sender) : View(context) {
    var mode = Mode.DRAW

    // Размер монитора ПК; уточняется по первому кадру
    private var monW = 1920f
    private var monH = 1080f

    @Volatile private var frame: Frame? = null

    // Вид: scale - пикселей экрана на пиксель монитора; (ox, oy) - точка монитора в левом верхнем углу экрана
    private var scale = 1f
    private var ox = 0f
    private var oy = 0f
    private var fitScale = 1f

    // Видео H.264 выводится на TextureView под этим видом; здесь рисуются только перо, курсор и подсказки
    private var videoView: TextureView? = null
    @Volatile private var surface: Surface? = null
    private val videoMatrix = Matrix()

    fun attachVideo(view: TextureView) {
        videoView = view
        view.surfaceTextureListener = object : TextureView.SurfaceTextureListener {
            override fun onSurfaceTextureAvailable(st: SurfaceTexture, w: Int, h: Int) {
                surface = Surface(st)
            }
            override fun onSurfaceTextureSizeChanged(st: SurfaceTexture, w: Int, h: Int) {}
            override fun onSurfaceTextureDestroyed(st: SurfaceTexture): Boolean {
                surface?.release()
                surface = null
                return true
            }
            override fun onSurfaceTextureUpdated(st: SurfaceTexture) {}
        }
        view.surfaceTexture?.let { surface = Surface(it) }
    }

    fun videoSurface(): Surface? = surface

    // Запрос видеопотока (24 байта): область, размер картинки (фиксированный, по размеру экрана), кбит/с и кадров в секунду
    fun videoRequest(kbps: Int, fps: Int): ByteArray {
        val x0 = ox.coerceAtLeast(0f)
        val y0 = oy.coerceAtLeast(0f)
        val x1 = (ox + width / scale).coerceAtMost(monW)
        val y1 = (oy + height / scale).coerceAtMost(monH)
        var ow = width.toFloat()
        var oh = height.toFloat()
        val limit = 1920f * 1080f
        if (ow * oh > limit) {
            val k = sqrt(limit / (ow * oh))
            ow *= k
            oh *= k
        }
        val b = ByteBuffer.allocate(24).order(ByteOrder.LITTLE_ENDIAN)
        b.putFloat(x0 / monW).putFloat(y0 / monH).putFloat((x1 - x0) / monW).putFloat((y1 - y0) / monH)
        b.putShort((ow.toInt() and 1.inv()).coerceIn(16, 1920).toShort())
        b.putShort((oh.toInt() and 1.inv()).coerceIn(16, 1920).toShort())
        b.putShort(kbps.coerceIn(300, 20000).toShort())
        b.put(fps.toByte())
        return b.array()
    }

    fun onVideoFrame(m: VideoMeta) =
        onFrame(Frame(null, m.x, m.y, m.w, m.h, m.cursorX, m.cursorY), SystemClock.uptimeMillis() - m.lagMs, m.monW.toFloat(), m.monH.toFloat())

    val zoom: Float get() = if (fitScale > 0f) scale / fitScale else 1f

    private var gesturing = false
    private var gDist = 1f
    private var gScale = 1f
    private var gAnchorX = 0f
    private var gAnchorY = 0f
    private var gAngle = 0f
    private var gAngleAcc = 0f

    // Касание тремя пальцами - отмена, четырьмя - возврат
    private var tapStart = 0L
    private var tapMax = 0
    private var tapMoved = false
    private val tapX = FloatArray(10)
    private val tapY = FloatArray(10)

    private var penDown = false
    private var lastX = 0f
    private var lastY = 0f
    private var downTime = 0L
    private var downX = 0f
    private var downY = 0f
    private var moved = false

    private val ink = ArrayList<InkPoint>()
    private val inkPaint = Paint().apply {
        color = Color.argb(230, 24, 24, 28)
        strokeWidth = 4f
        strokeCap = Paint.Cap.ROUND
        isAntiAlias = true
    }
    private val bitmapPaint = Paint().apply { isFilterBitmap = true }
    private val cursorFill = Paint().apply { color = Color.WHITE; isAntiAlias = true }
    private val cursorLine = Paint().apply {
        color = Color.rgb(20, 20, 24); style = Paint.Style.STROKE; strokeWidth = 2.5f; isAntiAlias = true
        strokeJoin = Paint.Join.ROUND
    }
    private val cursorPath = Path().apply {
        moveTo(0f, 0f); lineTo(0f, 16f); lineTo(4f, 12.5f); lineTo(7f, 19f); lineTo(9.5f, 18f)
        lineTo(6.5f, 11.7f); lineTo(11.5f, 11.7f); close()
    }

    init {
        // Пока рисуем, экран не гаснет
        keepScreenOn = true
    }

    fun fit() {
        fitScale = min(width / monW, height / monH)
        scale = fitScale
        ox = -(width / scale - monW) / 2
        oy = -(height / scale - monH) / 2
        invalidate()
    }

    // Запрос к ПК (24 байта): видимая область монитора и размер картинки в пикселях экрана
    fun screenRequest(): ByteArray {
        val x0 = ox.coerceAtLeast(0f)
        val y0 = oy.coerceAtLeast(0f)
        val x1 = (ox + width / scale).coerceAtMost(monW)
        val y1 = (oy + height / scale).coerceAtMost(monH)
        val b = ByteBuffer.allocate(24).order(ByteOrder.LITTLE_ENDIAN)
        b.putFloat(x0 / monW).putFloat(y0 / monH).putFloat((x1 - x0) / monW).putFloat((y1 - y0) / monH)
        b.putShort(((x1 - x0) * scale).toInt().coerceIn(16, 2560).toShort())
        b.putShort(((y1 - y0) * scale).toInt().coerceIn(16, 1440).toShort())
        b.put(AppSettings.quality.toByte())
        return b.array()
    }

    // sentAt - момент, когда был отправлен запрос этого кадра
    fun onFrame(f: Frame, sentAt: Long, w: Float, h: Float) {
        frame = f
        pruneInk(sentAt - INK_MARGIN_MS)
        if (w != monW || h != monH) {
            monW = w; monH = h
            post { fit() }
        }
        postInvalidate()
    }

    private fun pruneInk(before: Long) {
        synchronized(ink) {
            val old = SystemClock.uptimeMillis() - INK_MAX_AGE_MS
            ink.removeAll { it.t < before || it.t < old }
            ink.firstOrNull()?.start = true
        }
    }

    private fun addInk(x: Float, y: Float, start: Boolean) {
        synchronized(ink) { ink.add(InkPoint(ox + x / scale, oy + y / scale, SystemClock.uptimeMillis(), start)) }
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) = fit()

    private fun nx(x: Float) = ((ox + x / scale) / monW).coerceIn(0f, 1f)
    private fun ny(y: Float) = ((oy + y / scale) / monH).coerceIn(0f, 1f)

    override fun onDraw(c: Canvas) {
        frame?.let {
            val dst = RectF(
                (it.x * monW - ox) * scale, (it.y * monH - oy) * scale,
                ((it.x + it.w) * monW - ox) * scale, ((it.y + it.h) * monH - oy) * scale,
            )
            val bmp = it.bmp
            if (bmp != null) {
                videoView?.alpha = 0f
                c.drawBitmap(bmp, null, dst, bitmapPaint)
            } else videoView?.let { v ->
                v.alpha = 1f
                videoMatrix.setScale(dst.width() / width, dst.height() / height)
                videoMatrix.postTranslate(dst.left, dst.top)
                v.setTransform(videoMatrix)
            }
        }
        frame?.let {
            // В режиме «Мышь» показываем, где сейчас курсор Windows
            if (mode == Mode.MOUSE && it.cursorX >= 0f) {
                val k = resources.displayMetrics.density * 1.1f
                c.save()
                c.translate((it.cursorX * monW - ox) * scale, (it.cursorY * monH - oy) * scale)
                c.scale(k, k)
                c.drawPath(cursorPath, cursorFill)
                c.drawPath(cursorPath, cursorLine)
                c.restore()
            }
        }
        synchronized(ink) {
            var prev: InkPoint? = null
            for (p in ink) {
                if (prev != null && !p.start) {
                    c.drawLine((prev.x - ox) * scale, (prev.y - oy) * scale, (p.x - ox) * scale, (p.y - oy) * scale, inkPaint)
                }
                prev = p
            }
        }
    }

    private fun pen(action: Byte, x: Float, y: Float, pressure: Float, tiltX: Int = 0, tiltY: Int = 0) =
        sender.pen(action, nx(x), ny(y), pressure, tiltX, tiltY)

    // Стилус всегда рисует; палец зависит от режима
    private fun stylus(e: MotionEvent): Boolean {
        val tilt = Math.toDegrees(e.getAxisValue(MotionEvent.AXIS_TILT).toDouble())
        val ori = e.getAxisValue(MotionEvent.AXIS_ORIENTATION).toDouble()
        val tx = (tilt * sin(ori)).toInt()
        val ty = (tilt * cos(ori)).toInt()
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                pen(DOWN, e.x, e.y, e.pressure, tx, ty)
                addInk(e.x, e.y, true)
            }
            MotionEvent.ACTION_MOVE -> {
                for (h in 0 until e.historySize) {
                    pen(DRAG, e.getHistoricalX(h), e.getHistoricalY(h), e.getHistoricalPressure(h), tx, ty)
                    addInk(e.getHistoricalX(h), e.getHistoricalY(h), false)
                }
                pen(DRAG, e.x, e.y, e.pressure, tx, ty)
                addInk(e.x, e.y, false)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> pen(UP, e.x, e.y, 0f, tx, ty)
            MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_ENTER -> pen(HOVER, e.x, e.y, 0f, tx, ty)
            MotionEvent.ACTION_HOVER_EXIT -> pen(LEAVE, e.x, e.y, 0f, tx, ty)
            else -> return false
        }
        invalidate()
        return true
    }

    private fun fingerDown(e: MotionEvent) {
        lastX = e.x; lastY = e.y; downX = e.x; downY = e.y
        downTime = e.eventTime; moved = false
        when (mode) {
            Mode.DRAW -> {
                pen(DOWN, e.x, e.y, AppSettings.fingerPressure)
                addInk(e.x, e.y, true)
                penDown = true
            }
            Mode.MOUSE -> sender.mouseMove(nx(e.x), ny(e.y))
        }
    }

    private fun fingerMove(e: MotionEvent) {
        if (hypot(e.x - downX, e.y - downY) > TAP_SLOP) moved = true
        lastX = e.x; lastY = e.y
        when (mode) {
            Mode.DRAW -> {
                for (h in 0 until e.historySize) {
                    pen(DRAG, e.getHistoricalX(h), e.getHistoricalY(h), AppSettings.fingerPressure)
                    addInk(e.getHistoricalX(h), e.getHistoricalY(h), false)
                }
                pen(DRAG, e.x, e.y, AppSettings.fingerPressure)
                addInk(e.x, e.y, false)
                invalidate()
            }
            Mode.MOUSE -> sender.mouseMove(nx(e.x), ny(e.y))
        }
    }

    // up = true: палец отпущен; false: отменён вторым пальцем (клик не нужен)
    private fun fingerEnd(e: MotionEvent?, up: Boolean) {
        when (mode) {
            Mode.DRAW -> if (penDown) {
                pen(UP, lastX, lastY, 0f)
                pen(LEAVE, lastX, lastY, 0f)
                penDown = false
            }
            Mode.MOUSE -> if (up && e != null && !moved) {
                val button = if (e.eventTime - downTime >= LONG_PRESS_MS) 1 else 0
                sender.mouseButton(button, true)
                sender.mouseButton(button, false)
            }
        }
    }

    private fun gestureStart(e: MotionEvent) {
        gDist = hypot(e.getX(0) - e.getX(1), e.getY(0) - e.getY(1)).coerceAtLeast(1f)
        gScale = scale
        val mx = (e.getX(0) + e.getX(1)) / 2
        val my = (e.getY(0) + e.getY(1)) / 2
        gAnchorX = ox + mx / scale
        gAnchorY = oy + my / scale
        gAngle = atan2(e.getY(1) - e.getY(0), e.getX(1) - e.getX(0))
        gAngleAcc = 0f
    }

    // Поворот двумя пальцами вращает холст в программе на ПК: шаг 15°, как у горячих клавиш Krita
    private fun gestureRotate(e: MotionEvent) {
        val a = atan2(e.getY(1) - e.getY(0), e.getX(1) - e.getX(0))
        var d = a - gAngle
        if (d > Math.PI) d -= (2 * Math.PI).toFloat()
        if (d < -Math.PI) d += (2 * Math.PI).toFloat()
        gAngle = a
        gAngleAcc += d
        val step = Math.toRadians(15.0).toFloat()
        while (gAngleAcc >= step) { sender.rotateCanvas(true); gAngleAcc -= step }
        while (gAngleAcc <= -step) { sender.rotateCanvas(false); gAngleAcc += step }
    }

    private fun gestureMove(e: MotionEvent) {
        val d = hypot(e.getX(0) - e.getX(1), e.getY(0) - e.getY(1))
        val mx = (e.getX(0) + e.getX(1)) / 2
        val my = (e.getY(0) + e.getY(1)) / 2
        scale = (gScale * d / gDist).coerceIn(fitScale * 0.9f, fitScale * 12f)
        ox = gAnchorX - mx / scale
        oy = gAnchorY - my / scale
        gestureRotate(e)
        invalidate()
    }

    private fun trackTap(e: MotionEvent) {
        for (i in 0 until e.pointerCount) {
            val id = e.getPointerId(i)
            if (id < tapX.size && hypot(e.getX(i) - tapX[id], e.getY(i) - tapY[id]) > 48f) tapMoved = true
        }
    }

    private fun rememberTap(e: MotionEvent, index: Int) {
        val id = e.getPointerId(index)
        if (id < tapX.size) { tapX[id] = e.getX(index); tapY[id] = e.getY(index) }
    }

    override fun onTouchEvent(e: MotionEvent): Boolean {
        if (e.getToolType(0) == MotionEvent.TOOL_TYPE_STYLUS) return stylus(e)
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                gesturing = false
                tapStart = e.eventTime; tapMax = 1; tapMoved = false
                rememberTap(e, 0)
                fingerDown(e)
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                tapMax = maxOf(tapMax, e.pointerCount)
                rememberTap(e, e.actionIndex)
                if (!gesturing) {
                    fingerEnd(null, false)
                    gesturing = true
                }
                if (e.pointerCount == 2) gestureStart(e)
            }
            MotionEvent.ACTION_MOVE -> {
                trackTap(e)
                when {
                    gesturing -> if (e.pointerCount >= 2) gestureMove(e)
                    else -> fingerMove(e)
                }
            }
            MotionEvent.ACTION_POINTER_UP -> if (gesturing && e.pointerCount == 3) {
                // Остались два пальца: пересчитать опору жеста
                gestureStart(e)
            }
            MotionEvent.ACTION_UP -> {
                if (!gesturing) fingerEnd(e, true)
                gesturing = false
                if (tapMax >= 3 && !tapMoved && e.eventTime - tapStart < 400) {
                    if (tapMax == 3) sender.undo() else sender.redo()
                }
                tapMax = 0
            }
            MotionEvent.ACTION_CANCEL -> { if (!gesturing) fingerEnd(e, false); gesturing = false }
            else -> return false
        }
        return true
    }

    override fun onGenericMotionEvent(e: MotionEvent): Boolean =
        if (e.getToolType(0) == MotionEvent.TOOL_TYPE_STYLUS) stylus(e) else false
}
