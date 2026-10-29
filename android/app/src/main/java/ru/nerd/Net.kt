package ru.nerd

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.wifi.WifiManager
import android.os.SystemClock
import java.io.DataInputStream
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.net.SocketTimeoutException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.Semaphore

internal const val PORT = 47800
internal const val SCREEN_PORT = 47801
internal const val DISCOVERY_PORT = 47802

// Настройки, которые меняются на экране «Настройки»
object AppSettings {
    @Volatile var quality = 60
    @Volatile var fingerPressure = 0.6f

    fun load(prefs: android.content.SharedPreferences) {
        quality = prefs.getInt("quality", 60)
        fingerPressure = prefs.getFloat("finger_pressure", 0.6f)
    }

    fun save(prefs: android.content.SharedPreferences) {
        prefs.edit().putInt("quality", quality).putFloat("finger_pressure", fingerPressure).apply()
    }
}

// Коды клавиш Windows
internal const val VK_CTRL = 0x11
internal const val VK_SHIFT = 0x10
internal const val VK_SPACE = 0x20
internal const val VK_Z = 0x5A
internal const val VK_E = 0x45
internal const val VK_BRACKET_LEFT = 0xDB
internal const val VK_BRACKET_RIGHT = 0xDD
private const val MAGIC: Byte = 0x4E
private const val CMD_MAGIC: Byte = 0x4F

// Те же действия, что в pc/src/main.rs
internal const val HOVER: Byte = 0
internal const val DOWN: Byte = 1
internal const val DRAG: Byte = 2
internal const val UP: Byte = 3
internal const val LEAVE: Byte = 4

// Сколько запросов кадра держим в полёте: ПК не ждёт, пока устройство закончит предыдущий
private const val IN_FLIGHT = 2

class Sender(host: String) {
    // Код сессии выдаёт ПК после подтверждения; пакеты без него ПК игнорирует
    @Volatile var tag = 0
    private val queue = LinkedBlockingQueue<ByteArray>()
    private val socket = DatagramSocket()
    private val address = InetAddress.getByName(host)

    init {
        Thread {
            while (true) {
                val b = queue.take()
                socket.send(DatagramPacket(b, b.size, address, PORT))
            }
        }.apply { isDaemon = true }.start()
    }

    // magic u8 | action u8 | код сессии u16 | x f32 | y f32 | pressure f32 | tilt_x i16 | tilt_y i16
    fun pen(action: Byte, x: Float, y: Float, pressure: Float, tiltX: Int, tiltY: Int) {
        val b = ByteBuffer.allocate(20).order(ByteOrder.LITTLE_ENDIAN)
        b.put(MAGIC).put(action).putShort(tag.toShort())
        b.putFloat(x).putFloat(y).putFloat(pressure)
        b.putShort(tiltX.toShort()).putShort(tiltY.toShort())
        queue.offer(b.array())
    }

    // CMD_MAGIC | kind u8 | код сессии u16 | x f32 | y f32 (kind 0: курсор)
    fun mouseMove(x: Float, y: Float) {
        val b = ByteBuffer.allocate(12).order(ByteOrder.LITTLE_ENDIAN)
        b.put(CMD_MAGIC).put(0).putShort(tag.toShort()).putFloat(x).putFloat(y)
        queue.offer(b.array())
    }

    // CMD_MAGIC | kind u8 (2: клавиша) | код сессии u16 | код клавиши Windows u16 | down u8
    fun key(vk: Int, down: Boolean) {
        queue.offer(byteArrayOf(CMD_MAGIC, 2, tag.toByte(), (tag shr 8).toByte(), vk.toByte(), (vk shr 8).toByte(), if (down) 1 else 0))
    }

    // Сочетание: клавиши нажимаются по порядку, отпускаются в обратном
    fun combo(vararg keys: Int) {
        keys.forEach { key(it, true) }
        keys.reversed().forEach { key(it, false) }
    }

    fun undo() = combo(VK_CTRL, VK_Z)
    fun redo() = combo(VK_CTRL, VK_SHIFT, VK_Z)
    fun rotateCanvas(clockwise: Boolean) = combo(VK_CTRL, if (clockwise) VK_BRACKET_RIGHT else VK_BRACKET_LEFT)

    // CMD_MAGIC | kind u8 (1: кнопка) | код сессии u16 | кнопка u8 (0 левая, 1 правая) | down u8
    fun mouseButton(button: Int, down: Boolean) {
        queue.offer(byteArrayOf(CMD_MAGIC, 1, tag.toByte(), (tag shr 8).toByte(), button.toByte(), if (down) 1 else 0))
    }
}

// Кадр экрана ПК и область монитора (доли 0..1), которую он показывает
class Frame(
    val bmp: Bitmap,
    val x: Float,
    val y: Float,
    val w: Float,
    val h: Float,
    // курсор Windows в долях монитора; меньше нуля - вне монитора
    val cursorX: Float,
    val cursorY: Float,
)

// Постоянные номер и ключ этого устройства: по ним ПК узнаёт, что устройство уже разрешено
class Identity(val id: ByteArray, val token: ByteArray, val name: String) {
    companion object {
        fun load(prefs: android.content.SharedPreferences): Identity {
            fun hex(b: ByteArray) = b.joinToString("") { "%02x".format(it) }
            fun unhex(s: String) = ByteArray(s.length / 2) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
            var id = prefs.getString("device_id", null)
            var token = prefs.getString("device_token", null)
            if (id == null || token == null) {
                val rnd = java.security.SecureRandom()
                id = hex(ByteArray(16).also { rnd.nextBytes(it) })
                token = hex(ByteArray(16).also { rnd.nextBytes(it) })
                prefs.edit().putString("device_id", id).putString("device_token", token).apply()
            }
            return Identity(unhex(id), unhex(token), "${android.os.Build.MANUFACTURER} ${android.os.Build.MODEL}")
        }
    }
}

enum class ConnState { CONNECTING, WAITING, CONNECTED, DENIED }

class MonitorOption(val num: Int, val w: Int, val h: Int)

class ScreenClient(
    context: Context,
    private val host: String,
    private val pad: PadView,
    private val sender: Sender,
    private val identity: Identity,
) {
    @Volatile private var running = true
    @Volatile private var socket: Socket? = null
    @Volatile var state = ConnState.CONNECTING
        private set
    @Volatile var statsText = ""
        private set
    @Volatile var monitors: List<MonitorOption> = emptyList()
        private set
    @Volatile var currentMonitor = 0
        private set
    @Volatile private var wantMonitor = -1

    // Без этого Wi-Fi уходит в энергосбережение и добавляет к задержке десятки миллисекунд
    private val wifiLock = runCatching {
        val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        val mode = if (android.os.Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY else WifiManager.WIFI_MODE_FULL_HIGH_PERF
        wifi.createWifiLock(mode, "nerd").apply { setReferenceCounted(false); acquire() }
    }.getOrNull()

    init {
        Thread { loop() }.apply { isDaemon = true; start() }
    }

    fun stop() {
        running = false
        socket?.close()
        runCatching { wifiLock?.release() }
    }

    // Выбор монитора на ПК; отправляется перед следующим запросом кадра
    fun selectMonitor(num: Int) {
        currentMonitor = num
        wantMonitor = num
    }

    // "NERD" | версия 2 | id 16 байт | ключ 16 байт | длина имени | имя
    private fun hello(): ByteArray {
        val name = identity.name.toByteArray().take(40).toByteArray()
        return "NERD".toByteArray() + byteArrayOf(2) + identity.id + identity.token + byteArrayOf(name.size.toByte()) + name
    }

    private fun loop() {
        while (running) {
            val s = Socket()
            var writer: Thread? = null
            try {
                socket = s
                state = ConnState.CONNECTING
                s.tcpNoDelay = true
                s.connect(InetSocketAddress(host, SCREEN_PORT), 3000)
                val input = DataInputStream(s.getInputStream().buffered(1 shl 16))
                val out = s.getOutputStream()
                out.write(hello())
                out.flush()

                // Пока человек на компьютере нажимает «Разрешить», ждём ответ (до минуты)
                state = ConnState.WAITING
                s.soTimeout = 70_000
                if (input.readUnsignedByte() == 0) {
                    state = ConnState.DENIED
                    running = false
                    break
                }
                val b0 = ByteArray(4)
                input.readFully(b0)
                val bb = ByteBuffer.wrap(b0).order(ByteOrder.LITTLE_ENDIAN)
                sender.tag = bb.short.toInt() and 0xFFFF
                currentMonitor = bb.get().toInt() and 0xFF
                val count = bb.get().toInt() and 0xFF
                val list = ArrayList<MonitorOption>()
                repeat(count) {
                    val m = ByteArray(5)
                    input.readFully(m)
                    val mb = ByteBuffer.wrap(m).order(ByteOrder.LITTLE_ENDIAN)
                    list.add(MonitorOption(mb.get().toInt() and 0xFF, mb.short.toInt() and 0xFFFF, mb.short.toInt() and 0xFFFF))
                }
                monitors = list
                s.soTimeout = 0

                val sentAt = ConcurrentLinkedQueue<Long>()
                val slots = Semaphore(IN_FLIGHT)
                writer = Thread {
                    try {
                        while (running) {
                            slots.acquire()
                            val m = wantMonitor
                            if (m >= 0) {
                                wantMonitor = -1
                                out.write(byteArrayOf(1, m.toByte()))
                            }
                            sentAt.add(SystemClock.uptimeMillis())
                            out.write(byteArrayOf(0) + pad.screenRequest())
                            out.flush()
                        }
                    } catch (e: Exception) {
                        // связь закрыта или поток прерван
                    }
                }.apply { isDaemon = true; start() }
                state = ConnState.CONNECTED

                val head = ByteArray(32)
                var frames = 0
                var bytes = 0L
                var rttSum = 0L
                var since = SystemClock.uptimeMillis()
                while (running) {
                    input.readFully(head)
                    val b = ByteBuffer.wrap(head).order(ByteOrder.LITTLE_ENDIAN)
                    val x = b.float; val y = b.float; val w = b.float; val h = b.float
                    val monW = b.short.toInt() and 0xFFFF
                    val monH = b.short.toInt() and 0xFFFF
                    val len = b.int
                    val cursorX = b.float; val cursorY = b.float
                    if (len <= 0 || len > 20_000_000) break
                    val data = ByteArray(len)
                    input.readFully(data)
                    val sent = sentAt.poll() ?: SystemClock.uptimeMillis()
                    slots.release()
                    val bmp = BitmapFactory.decodeByteArray(data, 0, len) ?: continue
                    val now = SystemClock.uptimeMillis()
                    pad.onFrame(Frame(bmp, x, y, w, h, cursorX, cursorY), sent, monW.toFloat(), monH.toFloat())

                    frames++; bytes += len; rttSum += now - sent
                    if (now - since >= 1000) {
                        statsText = "${frames * 1000 / (now - since)} к/с · ${rttSum / frames} мс · ${bytes / frames / 1024} КБ"
                        frames = 0; bytes = 0; rttSum = 0; since = now
                    }
                }
            } catch (e: Exception) {
                // нет связи: пробуем снова
            } finally {
                if (state != ConnState.DENIED) state = ConnState.CONNECTING
                statsText = ""
                sender.tag = 0
                writer?.interrupt()
                runCatching { s.close() }
            }
            if (running) Thread.sleep(1000)
        }
    }
}

class FoundPc(val name: String, val ip: String)

// Ищет компьютеры с запущенной программой: рассылает «NERD?», ПК отвечает «NERD!имя»
class Discovery(context: Context) {
    @Volatile var found: List<FoundPc> = emptyList()
        private set
    @Volatile private var running = true
    private val seen = ConcurrentHashMap<String, Pair<String, Long>>()
    private val targets = broadcastAddresses(context)

    init {
        Thread { loop() }.apply { isDaemon = true; start() }
    }

    fun stop() {
        running = false
    }

    private fun loop() {
        val socket = DatagramSocket()
        socket.broadcast = true
        socket.soTimeout = 300
        val ping = "NERD?".toByteArray()
        val buf = ByteArray(128)
        var lastPing = 0L
        while (running) {
            var now = SystemClock.uptimeMillis()
            if (now - lastPing >= 1000) {
                for (t in targets) runCatching { socket.send(DatagramPacket(ping, ping.size, t, DISCOVERY_PORT)) }
                lastPing = now
            }
            try {
                val p = DatagramPacket(buf, buf.size)
                socket.receive(p)
                val msg = String(p.data, 0, p.length)
                if (msg.startsWith("NERD!")) {
                    seen[p.address.hostAddress ?: continue] = msg.substring(5) to SystemClock.uptimeMillis()
                }
            } catch (e: SocketTimeoutException) {
                // ответа нет, ждём дальше
            }
            now = SystemClock.uptimeMillis()
            found = seen.filter { now - it.value.second < 4000 }.map { FoundPc(it.value.first, it.key) }
        }
        socket.close()
    }
}

// Общий широковещательный адрес и адрес вашей Wi-Fi сети (на некоторых устройствах общий не доходит)
private fun broadcastAddresses(context: Context): List<InetAddress> {
    val list = mutableListOf(InetAddress.getByName("255.255.255.255"))
    runCatching {
        val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        @Suppress("DEPRECATION")
        val dhcp = wifi.dhcpInfo
        val mask = if (dhcp.netmask != 0) dhcp.netmask else 0x00FFFFFF
        if (dhcp.ipAddress != 0) {
            val b = (dhcp.ipAddress and mask) or mask.inv()
            list.add(InetAddress.getByAddress(byteArrayOf(b.toByte(), (b shr 8).toByte(), (b shr 16).toByte(), (b shr 24).toByte())))
        }
    }
    return list
}
