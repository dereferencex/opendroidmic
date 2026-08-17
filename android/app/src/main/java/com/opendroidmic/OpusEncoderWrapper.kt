package com.opendroidmic

import android.util.Log
import io.github.jaredmdobson.concentus.OpusEncoder
import io.github.jaredmdobson.concentus.OpusException
import io.github.jaredmdobson.concentus.OpusApplication
import io.github.jaredmdobson.concentus.OpusBandwidth

class OpusEncoderWrapper(
    private val sampleRate: Int,
    private val channels: Int,
    private val bitrate: Int
) {
    private var encoder: OpusEncoder? = null
    private val frameSize: Int = 960

    init {
        try {
            encoder = OpusEncoder(sampleRate, channels, OpusApplication.OPUS_APPLICATION_VOIP).apply {
                setBitrate(bitrate)
                setBandwidth(OpusBandwidth.OPUS_BANDWIDTH_WIDEBAND)
            }
            Log.d("OpusEncoder", "Initialized: ${sampleRate}Hz ${channels}ch ${bitrate}bps")
        } catch (e: OpusException) {
            Log.e("OpusEncoder", "Failed to init", e)
        }
    }

    fun encode(pcmData: ShortArray, samplesRead: Int): ByteArray? {
        val enc = encoder ?: return null
        return try {
            val maxPacketSize = 4000
            val output = ByteArray(maxPacketSize)
            val encodedSize = enc.encode(pcmData, 0, samplesRead, output, 0, maxPacketSize)
            if (encodedSize > 0) {
                output.copyOf(encodedSize)
            } else {
                null
            }
        } catch (e: OpusException) {
            Log.e("OpusEncoder", "Encode error", e)
            null
        }
    }

    fun release() {
        encoder?.resetState()
        encoder = null
    }
}
