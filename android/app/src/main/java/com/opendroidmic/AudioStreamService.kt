package com.opendroidmic

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.Binder
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.*
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

class AudioStreamService : Service() {
    companion object {
        private const val TAG = "AudioStreamService"
        private const val CHANNEL_ID = "opendroidmic_stream"
        private const val NOTIFICATION_ID = 1
        private const val SAMPLE_RATE = 48000
        private const val CHANNEL_COUNT = 1
        private const val FRAME_SIZE = 960
        private const val BITRATE = 32000
        private const val HELLO_ACK_TIMEOUT_MS = 3000L
        private const val PING_INTERVAL_MS = 5000L
        private const val PONG_TIMEOUT_MS = 5000L
        const val MAX_RECONNECT_ATTEMPTS = 10
        private const val BASE_RECONNECT_DELAY_MS = 500L
        private const val MAX_RECONNECT_DELAY_MS = 8000L
    }

    object State {
        const val DISCONNECTED = 0L
        const val CONNECTING = 1L
        const val WAITING_ACK = 2L
        const val CONNECTED = 3L
        const val STREAMING = 4L
        const val RECONNECTING = 5L
        const val ERROR = 6L
    }

    private val binder = LocalBinder()
    private var streamJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    val isStreaming = AtomicBoolean(false)
    val packetsSent = AtomicInteger(0)
    val packetsLost = AtomicInteger(0)
    val connectionState = AtomicLong(State.DISCONNECTED)
    val currentAudioLevel = AtomicInteger(0)
    val reconnectAttempts = AtomicInteger(0)
    val errorMessage = AtomicLong(0)

    private var host: String = ""
    private var port: Int = 0

    inner class LocalBinder : Binder() {
        fun getService(): AudioStreamService = this@AudioStreamService
    }

    override fun onBind(intent: Intent): IBinder = binder

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        return START_STICKY
    }

    fun startStreaming(host: String, port: Int) {
        if (isStreaming.get()) return

        this.host = host
        this.port = port

        val notification = buildNotification("Streaming to $host:$port")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }

        isStreaming.set(true)
        packetsSent.set(0)
        packetsLost.set(0)
        reconnectAttempts.set(0)
        errorMessage.set(0)
        connectionState.set(State.CONNECTING)

        streamJob = scope.launch {
            try {
                streamWithReconnect(host, port)
            } catch (e: CancellationException) {
                Log.d(TAG, "Stream cancelled")
            } catch (e: Exception) {
                Log.e(TAG, "Streaming error", e)
                connectionState.set(State.ERROR)
                isStreaming.set(false)
            }
        }
    }

    fun stopStreaming() {
        isStreaming.set(false)
        streamJob?.cancel()
        streamJob = null
        connectionState.set(State.DISCONNECTED)
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private suspend fun streamWithReconnect(host: String, port: Int) {
        while (isStreaming.get()) {
            try {
                streamAudio(host, port)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "Connection failed: ${e.message}")
            }

            if (!isStreaming.get()) break

            val attempt = reconnectAttempts.incrementAndGet()
            if (attempt > MAX_RECONNECT_ATTEMPTS) {
                Log.e(TAG, "Max reconnect attempts reached")
                connectionState.set(State.ERROR)
                isStreaming.set(false)
                break
            }

            connectionState.set(State.RECONNECTING)
            val delay = (BASE_RECONNECT_DELAY_MS * (1L shl (attempt - 1).coerceAtMost(4)))
                .coerceAtMost(MAX_RECONNECT_DELAY_MS)
            Log.d(TAG, "Reconnect attempt $attempt in ${delay}ms")
            delay(delay)
        }
    }

    private suspend fun streamAudio(host: String, port: Int) = withContext(Dispatchers.IO) {
        val bufferSize = AudioRecord.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT
        )

        val audioRecord = AudioRecord(
            MediaRecorder.AudioSource.MIC,
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
            bufferSize
        )

        val socket = DatagramSocket()
        socket.soTimeout = 1
        val address = InetAddress.getByName(host)

        val encoder = OpusEncoderWrapper(SAMPLE_RATE, CHANNEL_COUNT, BITRATE)
        val sessionToken = System.currentTimeMillis()
        val readBuffer = ShortArray(FRAME_SIZE)

        try {
            val helloPacket = Protocol.createHello(sessionToken)
            socket.send(DatagramPacket(helloPacket, helloPacket.size, address, port))
            Log.d(TAG, "Sent Hello to $host:$port (token: ${sessionToken.toString(16)})")

            connectionState.set(State.WAITING_ACK)

            var ackReceived = false
            val ackDeadline = System.currentTimeMillis() + HELLO_ACK_TIMEOUT_MS
            val recvBuf = ByteArray(Protocol.MAX_PACKET_SIZE)

            socket.soTimeout = 500
            while (!ackReceived && System.currentTimeMillis() < ackDeadline) {
                try {
                    val recvPacket = DatagramPacket(recvBuf, recvBuf.size)
                    socket.receive(recvPacket)
                    val parsed = Protocol.parse(recvBuf, recvPacket.length)
                    if (parsed != null && parsed.type == Protocol.TYPE_HELLO_ACK) {
                        ackReceived = true
                        Log.d(TAG, "Received HelloAck")
                    }
                } catch (_: java.net.SocketTimeoutException) {
                    // keep waiting
                }
            }
            socket.soTimeout = 1

            if (!ackReceived) {
                Log.w(TAG, "HelloAck timeout")
                throw java.io.IOException("HelloAck timeout")
            }

            audioRecord.startRecording()
            connectionState.set(State.CONNECTED)

            var sequence = 0
            var lastPingTime = System.currentTimeMillis()
            var lastPongTime = System.currentTimeMillis()
            var waitingPong = false

            while (isStreaming.get() && isActive) {
                val read = audioRecord.read(readBuffer, 0, FRAME_SIZE)
                if (read <= 0) continue

                var sum = 0L
                for (i in 0 until read) {
                    sum += Math.abs(readBuffer[i].toInt())
                }
                val avgLevel = (sum / read * 100 / 32768).toInt().coerceIn(0, 100)
                currentAudioLevel.set(avgLevel)

                val opusFrame = encoder.encode(readBuffer, read)
                if (opusFrame != null) {
                    val timestamp = sequence * 20
                    val packet = Protocol.createAudio(sequence, timestamp, opusFrame)
                    socket.send(DatagramPacket(packet, packet.size, address, port))
                    packetsSent.incrementAndGet()
                    sequence++
                }

                if (connectionState.get() == State.CONNECTED) {
                    connectionState.set(State.STREAMING)
                    updateNotification()
                }

                // Update notification every 2 seconds
                if (sequence % 100 == 0 && sequence > 0) {
                    updateNotification()
                }

                // Send periodic pings for keepalive
                val now = System.currentTimeMillis()
                if (now - lastPingTime >= PING_INTERVAL_MS && !waitingPong) {
                    val pingPacket = Protocol.createPing(sequence, (sequence * 20))
                    socket.send(DatagramPacket(pingPacket, pingPacket.size, address, port))
                    lastPingTime = now
                    waitingPong = true
                    lastPongTime = now
                }

                // Check pong timeout
                if (waitingPong && now - lastPongTime >= PONG_TIMEOUT_MS) {
                    Log.w(TAG, "Pong timeout - connection lost")
                    packetsLost.incrementAndGet()
                    throw java.io.IOException("Pong timeout")
                }

                // Drain any incoming packets (pong responses) - non-blocking
                try {
                    val recvPacket = DatagramPacket(recvBuf, recvBuf.size)
                    socket.receive(recvPacket)
                    val parsed = Protocol.parse(recvBuf, recvPacket.length)
                    if (parsed != null) {
                        when (parsed.type) {
                            Protocol.TYPE_PONG -> {
                                waitingPong = false
                                lastPongTime = now
                            }
                            Protocol.TYPE_STOP -> {
                                Log.d(TAG, "Stop received from server")
                                isStreaming.set(false)
                                return@withContext
                            }
                        }
                    }
                } catch (_: java.net.SocketTimeoutException) {
                    // no packet available
                }
            }
        } finally {
            try {
                val stopPacket = Protocol.createStop(sessionToken)
                socket.send(DatagramPacket(stopPacket, stopPacket.size, address, port))
            } catch (_: Exception) {}

            audioRecord.stop()
            audioRecord.release()
            socket.close()
            encoder.release()
        }
    }

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Audio Streaming",
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = "OpenDroidMic audio streaming notification"
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(channel)
    }

    private fun buildNotification(text: String): Notification {
        val intent = Intent(this, MainActivity::class.java)
        val pendingIntent = PendingIntent.getActivity(
            this, 0, intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        val stopIntent = Intent(this, MainActivity::class.java).apply {
            action = "ACTION_STOP"
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val stopPendingIntent = PendingIntent.getActivity(
            this, 1, stopIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        val sent = packetsSent.get()
        val contentText = if (sent > 0) {
            "$text  •  $sent packets sent"
        } else {
            text
        }

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("OpenDroidMic")
            .setContentText(contentText)
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentIntent(pendingIntent)
            .addAction(android.R.drawable.ic_media_pause, "Stop", stopPendingIntent)
            .setOngoing(true)
            .setShowWhen(false)
            .build()
    }

    private fun updateNotification() {
        val notification = buildNotification("Streaming to $host:$port")
        val manager = getSystemService(NotificationManager::class.java)
        manager.notify(NOTIFICATION_ID, notification)
    }

    override fun onDestroy() {
        super.onDestroy()
        scope.cancel()
    }
}
