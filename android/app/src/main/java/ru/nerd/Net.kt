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
internal const val JPEG_QUALITY = 60
private const val MAGIC: Byte = 0x4E
private const val CMD_MAGIC: Byte = 0x4F

// Те же действия, что в pc/src/main.rs
internal const val HOVER: Byte = 0
internal const val DOWN: Byte = 1
internal const val DRAG: Byte = 2
internal const val UP: Byte = 3
internal const val LEAVE: Byte = 4

// Сколько запросов кадра держим в полёте: ПК не ждёт, пока телефон закончит предыдущий
private const val IN_FLIGHT = 2

class Sender(host: String) {
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

    // magic u8 | action u8 | buttons u8 | pad u8 | x f32 | y f32 | pressure f32 | tilt_x i16 | tilt_y i16
    fun pen(action: Byte, x: Float, y: Float, pressure: Float, tiltX: Int, tiltY: Int) {
        val b = ByteBuffer.allocate(20).order(ByteOrder.LITTLE_ENDIAN)
        b.put(MAGIC).put(action).put(0).put(0)
        b.putFloat(x).putFloat(y).putFloat(pressure)
        b.putShort(tiltX.toShort()).putShort(tiltY.toShort())
        queue.offer(b.array())
    }

    // CMD_MAGIC | kind u8 | pad | pad | x f32 | y f32 (kind 0: курсор)
    fun mouseMove(x: Float, y: Float) {
        val b = ByteBuffer.allocate(12).order(ByteOrder.LITTLE_ENDIAN)
        b.put(CMD_MAGIC).put(0).put(0).put(0).putFloat(x).putFloat(y)
        queue.offer(b.array())
    }

    // CMD_MAGIC | kind u8 (1: кнопка) | кнопка u8 (0 левая, 1 правая) | down u8
    fun mouseButton(button: Int, down: Boolean) {
        queue.offer(byteArrayOf(CMD_MAGIC, 1, button.toByte(), if (down) 1 else 0))
    }
}

// Кадр экрана ПК и область монитора (доли 0..1), которую он показывает
class Frame(val bmp: Bitmap, val x: Float, val y: Float, val w: Float, val h: Float)

class ScreenClient(private val host: String, private val pad: PadView) {
    @Volatile private var running = true
    @Volatile private var socket: Socket? = null
    @Volatile var connected = false
        private set
    @Volatile var statsText = ""
        private set

    init {
        Thread { loop() }.apply { isDaemon = true; start() }
    }

    fun stop() {
        running = false
        socket?.close()
    }

    private fun loop() {
        while (running) {
            val s = Socket()
            var writer: Thread? = null
            try {
                socket = s
                s.tcpNoDelay = true
                s.connect(InetSocketAddress(host, SCREEN_PORT), 3000)
                val input = DataInputStream(s.getInputStream().buffered(1 shl 16))
                val out = s.getOutputStream()
                val sentAt = ConcurrentLinkedQueue<Long>()
                val slots = Semaphore(IN_FLIGHT)
                writer = Thread {
                    try {
                        while (running) {
                            slots.acquire()
                            sentAt.add(SystemClock.uptimeMillis())
                            out.write(pad.screenRequest())
                            out.flush()
                        }
                    } catch (e: Exception) {
                        // связь закрыта или поток прерван
                    }
                }.apply { isDaemon = true; start() }
                connected = true

                val head = ByteArray(24)
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
                    if (len <= 0 || len > 20_000_000) break
                    val data = ByteArray(len)
                    input.readFully(data)
                    val sent = sentAt.poll() ?: SystemClock.uptimeMillis()
                    slots.release()
                    val bmp = BitmapFactory.decodeByteArray(data, 0, len) ?: continue
                    val now = SystemClock.uptimeMillis()
                    pad.onFrame(Frame(bmp, x, y, w, h), sent, monW.toFloat(), monH.toFloat())

                    frames++; bytes += len; rttSum += now - sent
                    if (now - since >= 1000) {
                        statsText = "${frames * 1000 / (now - since)} к/с · ${rttSum / frames} мс · ${bytes / frames / 1024} КБ"
                        frames = 0; bytes = 0; rttSum = 0; since = now
                    }
                }
            } catch (e: Exception) {
                // нет связи: пробуем снова
            } finally {
                connected = false
                statsText = ""
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

// Общий широковещательный адрес и адрес вашей Wi-Fi сети (на некоторых телефонах общий не доходит)
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
