package com.opendroidmic

import java.nio.ByteBuffer
import java.nio.ByteOrder

object Protocol {
    val MAGIC = byteArrayOf(0x4F, 0x44, 0x4D, 0x43) // "ODMC"
    const val PROTOCOL_VERSION: Byte = 1
    const val HEADER_SIZE = 20
    const val MAX_PACKET_SIZE = 1500

    const val TYPE_HELLO: Byte = 0
    const val TYPE_HELLO_ACK: Byte = 1
    const val TYPE_AUDIO: Byte = 2
    const val TYPE_PING: Byte = 3
    const val TYPE_PONG: Byte = 4
    const val TYPE_STOP: Byte = 5
    const val TYPE_ERROR: Byte = 6

    fun createHeader(
        type: Byte,
        sequence: Int,
        timestamp: Int,
        payloadLength: Int
    ): ByteArray {
        val header = ByteArray(HEADER_SIZE)
        MAGIC.copyInto(header, 0)
        header[4] = PROTOCOL_VERSION
        header[5] = type
        header[6] = 0 // flags
        header[7] = 0 // reserved
        ByteBuffer.wrap(header, 8, 4).order(ByteOrder.LITTLE_ENDIAN).putInt(sequence)
        ByteBuffer.wrap(header, 12, 4).order(ByteOrder.LITTLE_ENDIAN).putInt(timestamp)
        ByteBuffer.wrap(header, 16, 2).order(ByteOrder.LITTLE_ENDIAN).putShort(payloadLength.toShort())
        ByteBuffer.wrap(header, 18, 2).order(ByteOrder.LITTLE_ENDIAN).putShort(0) // checksum
        return header
    }

    fun createHello(sessionToken: Long): ByteArray {
        val payload = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putLong(sessionToken).array()
        return createHeader(TYPE_HELLO, 0, 0, payload.size) + payload
    }

    fun createAudio(sequence: Int, timestamp: Int, opusFrame: ByteArray): ByteArray {
        return createHeader(TYPE_AUDIO, sequence, timestamp, opusFrame.size) + opusFrame
    }

    fun createPing(sequence: Int, timestamp: Int): ByteArray {
        val payload = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN)
            .putInt(sequence)
            .putInt(timestamp)
            .array()
        return createHeader(TYPE_PING, sequence, timestamp, payload.size) + payload
    }

    fun createStop(sessionToken: Long): ByteArray {
        val payload = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putLong(sessionToken).array()
        return createHeader(TYPE_STOP, 0, 0, payload.size) + payload
    }

    data class ParsedPacket(
        val type: Byte,
        val sequence: Int,
        val timestamp: Int,
        val payload: ByteArray
    ) {
        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other !is ParsedPacket) return false
            return type == other.type && sequence == other.sequence && timestamp == other.timestamp && payload.contentEquals(other.payload)
        }
        override fun hashCode(): Int {
            var result = type.toInt()
            result = 31 * result + sequence
            result = 31 * result + timestamp
            result = 31 * result + payload.contentHashCode()
            return result
        }
    }

    fun parse(data: ByteArray, length: Int): ParsedPacket? {
        if (length < HEADER_SIZE) return null

        val magic = data.sliceArray(0..3)
        if (!magic.contentEquals(MAGIC)) return null
        if (data[4] != PROTOCOL_VERSION) return null

        val type = data[5]
        val sequence = ByteBuffer.wrap(data, 8, 4).order(ByteOrder.LITTLE_ENDIAN).int
        val timestamp = ByteBuffer.wrap(data, 12, 4).order(ByteOrder.LITTLE_ENDIAN).int
        val payloadLen = ByteBuffer.wrap(data, 16, 2).order(ByteOrder.LITTLE_ENDIAN).short.toInt() and 0xFFFF

        if (length < HEADER_SIZE + payloadLen) return null

        val payload = data.sliceArray(HEADER_SIZE until HEADER_SIZE + payloadLen)
        return ParsedPacket(type, sequence, timestamp, payload)
    }
}
