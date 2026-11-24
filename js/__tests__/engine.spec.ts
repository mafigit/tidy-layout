import { readFile } from 'node:fs/promises'
import { beforeAll, describe, expect, it } from 'vitest'
import { LayoutEngineError, loadLayoutEngine } from '../index'
import type { LayoutEngine, SnapRequest } from '../index'

// Runs the real engine: pkg/tidy_layout.wasm, built by `bin/build`.
let engine: LayoutEngine
beforeAll(async () => {
  engine = await loadLayoutEngine(await readFile(new URL('../../pkg/tidy_layout.wasm', import.meta.url)))
})

const card = (id: string, x: number, y: number) => ({ id, x, y, width: 220, height: 120 })
const request = (wanted: { x: number; y: number }, connected = false): SnapRequest => ({
  cards: [card('a', 0, 0), card('b', 600, 0)],
  connections: connected ? [{ from: 'a', to: 'b' }] : [],
  moving: 'b',
  wanted,
  cellSize: 20,
})

describe('the layout engine, through WebAssembly', () => {
  it('takes a free place on the grid', () => {
    expect(engine.snap(request({ x: 407, y: 293 }))).toEqual({ x: 400, y: 300, ring: 0 })
  })

  it('moves a card dropped on another one to the nearest place with its gap', () => {
    expect(engine.snap(request({ x: 200, y: 0 }))).toMatchObject({ x: 240, y: 0 })
    expect(engine.snap(request({ x: 200, y: 0 }, true))).toMatchObject({ x: 280, y: 0 })
  })

  it('leaves room for a label between connected cards', () => {
    const labelled = { ...request({ x: 230, y: 0 }), connections: [{ from: 'a', to: 'b', label: { width: 150, height: 16 } }] }
    expect(engine.snap(labelled)).toMatchObject({ x: 380, y: 0 })
  })

  it('throws what the engine reports', () => {
    expect(() => engine.snap({ ...request({ x: 0, y: 0 }), moving: 'nope' })).toThrow(
      new LayoutEngineError('unknown card: nope'),
    )
  })

  it('carries text outside ASCII and a diagram of 500 cards', () => {
    const cards = Array.from({ length: 500 }, (_, i) => card(`Käse ${i} 書`, (i % 25) * 300, Math.floor(i / 25) * 200))
    const place = engine.snap({ cards, connections: [], moving: 'Käse 7 書', wanted: { x: 0, y: 0 }, cellSize: 20 })
    // The top-left card sits at 0,0: the nearest free place is somewhere else, on the grid.
    expect(place.ring).toBeGreaterThan(0)
    expect(Math.abs(place.x % 20) + Math.abs(place.y % 20)).toBe(0)
  })

  it('routes a connector around a card that sits between its ends', () => {
    const cards = [card('a', 0, 0), card('between', 300, 0), card('c', 600, 0)]
    const { routes, labels } = engine.route({
      cards,
      connections: [
        { from: 'a', to: 'c' },
        { from: 'a', to: 'between', label: { width: 60, height: 22 } },
      ],
    })
    const [around, direct] = routes
    // Only the second connection has a label: it sits above its connector, clear of the cards.
    expect(labels).toEqual([null, { x: 230, y: 32 }])
    // Next to each other: a straight line between the middles of the facing sides.
    expect(direct).toEqual([
      { x: 220, y: 60 },
      { x: 300, y: 60 },
    ])
    // Around the card between: more than one piece, none of them through it.
    expect(around!.length).toBeGreaterThan(2)
    for (let i = 1; i < around!.length; i++) {
      const [p, q] = [around![i - 1]!, around![i]!]
      const through = Math.min(p.x, q.x) < 520 && Math.max(p.x, q.x) > 300 && Math.min(p.y, q.y) < 120 && Math.max(p.y, q.y) > 0
      expect(through).toBe(false)
    }
  })

  it('refuses to route cards that overlap', () => {
    expect(() => engine.route({ cards: [card('a', 0, 0), card('b', 100, 0)], connections: [] })).toThrow(
      new LayoutEngineError('cards a and b overlap or touch'),
    )
  })

  it('arranges a diagram: own sizes, the 20 px gap, the same top-left corner', () => {
    const { cards, seed } = engine.arrange({
      cards: [card('a', 100, 40), card('b', 100, 40), card('c', 500, 300)],
      connections: [
        { from: 'a', to: 'b' },
        { from: 'a', to: 'b' },
        { from: 'a', to: 'b' },
        { from: 'b', to: 'c', label: { width: 90, height: 22 } },
      ],
    })
    expect(cards.map((c) => c.id)).toEqual(['a', 'b', 'c'])
    expect([1, 2, 3]).toContain(seed)
    // Three connections to one card: 160 high. The other card keeps its size.
    expect(cards.map((c) => [c.width, c.height])).toEqual([
      [220, 160],
      [220, 160],
      [220, 120],
    ])
    expect(Math.min(...cards.map((c) => c.x))).toBe(100)
    expect(Math.min(...cards.map((c) => c.y))).toBe(40)
    for (const [i, p] of cards.entries()) {
      for (const q of cards.slice(i + 1)) {
        const gap = Math.max(q.x - (p.x + p.width), p.x - (q.x + q.width), q.y - (p.y + p.height), p.y - (q.y + q.height))
        expect(gap).toBeGreaterThanOrEqual(20)
      }
    }
  })

  it('refuses to arrange with an unknown card', () => {
    expect(() => engine.arrange({ cards: [card('a', 0, 0)], connections: [{ from: 'a', to: 'nope' }] })).toThrow(
      new LayoutEngineError('unknown card: nope'),
    )
  })

  it('gives back the memory it used', () => {
    const before = engine.snap(request({ x: 200, y: 0 }))
    for (let i = 0; i < 5000; i++) engine.snap(request({ x: 200 + (i % 7), y: i % 5 }))
    expect(engine.snap(request({ x: 200, y: 0 }))).toEqual(before)
  })
})
