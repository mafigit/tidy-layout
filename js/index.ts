// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The JavaScript side of the layout engine (src/wasm.rs is the other): requests and answers
// travel as JSON text through the WebAssembly module's memory.

export interface Card {
  id: string
  /** Top-left corner. */
  x: number
  y: number
  width: number
  height: number
}

export interface Connection {
  from: string
  to: string
  /** The size of the label drawn on the connection, if it has one: it gets room between cards. */
  label?: { width: number; height: number }
}

/** "Where does this card go if it is dropped here?" */
export interface SnapRequest {
  /** Every card on the diagram, the moving one included, where it is now. */
  cards: Card[]
  connections: Connection[]
  /** Id of the card being moved. */
  moving: string
  /** Where its top-left corner is wanted. */
  wanted: { x: number; y: number }
  /** Step of the grid places lie on. Without it, about the size of a card. */
  cellSize?: number
}

export interface SnapPlace {
  /** The top-left corner the card gets: on the grid, keeping its gap to every other card. */
  x: number
  y: number
  /** How many grid rings away from the wanted place that is; 0 is the wanted place itself. */
  ring: number
}

/** "How do the connectors run?" */
export interface RouteRequest {
  /** Every card on the diagram, where it is. Cards must neither overlap nor touch. */
  cards: Card[]
  connections: Connection[]
}

/** The points a connector runs through, from the border of its first card to that of its second. */
export type Route = { x: number; y: number }[]

export interface Routes {
  /** A route around the cards for every connection, in the order of the request. */
  routes: Route[]
  /**
   * Per connection: the top-left corner of its label's box, where the label covers the least.
   * `null` for a connection without a label.
   */
  labels: ({ x: number; y: number } | null)[]
}

/** "Arrange the diagram." */
export interface ArrangeRequest {
  /** Every card on the diagram, where it is now. Cards may overlap: they are being moved. */
  cards: Card[]
  connections: Connection[]
}

export interface ArrangedCard {
  id: string
  /** The top-left corner the card gets. */
  x: number
  y: number
  /** The size the card gets: its own, larger for a card with many connections. */
  width: number
  height: number
}

export interface ArrangeResponse {
  /** Every card, in the order of the request. */
  cards: ArrangedCard[]
  /** The seed of TALA's pipeline the places come from: 1, 2 or 3. */
  seed: number
}

export interface LayoutEngine {
  /** The place nearest to the wanted one where the card keeps its gap to all others. */
  snap(request: SnapRequest): SnapPlace
  /**
   * Routes around the cards and places for the labels, for every connection in the order of the
   * request. A connection from a card to itself gets an empty route and no label place.
   */
  route(request: RouteRequest): Routes
  /**
   * Places for all cards as TALA arranges them, every two cards at least 20 apart. The top-left
   * corner of the box around all cards stays where it was.
   */
  arrange(request: ArrangeRequest): ArrangeResponse
}

export class LayoutEngineError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'LayoutEngineError'
  }
}

interface EngineExports {
  memory: WebAssembly.Memory
  alloc(length: number): number
  dealloc(pointer: number, length: number): void
  snap(pointer: number, length: number): bigint
  route(pointer: number, length: number): bigint
  arrange(pointer: number, length: number): bigint
}

/**
 * Start the engine from its `.wasm` file: the bytes, or a `fetch` of it. In the web app:
 * `loadLayoutEngine(fetch(new URL('tidy-layout/engine.wasm', import.meta.url)))`.
 */
export async function loadLayoutEngine(
  source: BufferSource | Response | PromiseLike<Response>,
): Promise<LayoutEngine> {
  const bytes =
    source instanceof ArrayBuffer || ArrayBuffer.isView(source)
      ? source
      : await (await source).arrayBuffer()
  const { instance } = await WebAssembly.instantiate(bytes as BufferSource)
  const engine = instance.exports as unknown as EngineExports
  const encoder = new TextEncoder()
  const decoder = new TextDecoder()

  function ask<Answer>(call: (pointer: number, length: number) => bigint, request: unknown): Answer {
    const input = encoder.encode(JSON.stringify(request))
    const pointer = engine.alloc(input.length)
    new Uint8Array(engine.memory.buffer, pointer, input.length).set(input)
    const packed = BigInt.asUintN(64, call(pointer, input.length))
    engine.dealloc(pointer, input.length)
    const answerPointer = Number(packed >> 32n)
    const answerLength = Number(packed & 0xffff_ffffn)
    // The memory may have grown during the call: look at its buffer only now.
    const text = decoder.decode(new Uint8Array(engine.memory.buffer, answerPointer, answerLength))
    engine.dealloc(answerPointer, answerLength)
    const answer = JSON.parse(text) as Answer | { error: string }
    if (typeof answer === 'object' && answer !== null && 'error' in answer) {
      throw new LayoutEngineError(answer.error)
    }
    return answer as Answer
  }

  return {
    snap: (request) => ask<SnapPlace>(engine.snap, request),
    route: (request) => ask<Routes>(engine.route, request),
    arrange: (request) => ask<ArrangeResponse>(engine.arrange, request),
  }
}
