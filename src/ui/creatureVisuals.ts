/**
 * How each creature is drawn (UI-only data, keyed by creature id; the engine defines the creature
 * itself): body outline, tissue shapes, airway centrelines per tube, and source glyphs.
 */
export interface CreatureVisual {
  /** body outline in drawing units (≈ cm; y up) */
  silhouette: [number, number][];
  /** extra shapes drawn in tissue tones (shade 0 black … 1 white) */
  parts?: { shade: number; pts: [number, number][]; stroke?: boolean }[];
  /** airway centrelines per tube name (tube nodes are distributed along the path); inView: false
   *  keeps the label out of the cross-section (it still names the tube's lane) */
  tubes: { tube: string; path: [number, number][]; label?: string; inView?: boolean }[];
  /** how much wider than life to draw the airway (narrow tubes stay legible) */
  tubeScale?: number;
  /** drawing extents */
  view: { x0: number; x1: number; y0: number; y1: number };
  /** sound sources to draw, keyed into the creature's source state */
  sources?: { key: string; at: [number, number]; label: string }[];
}

export const CREATURE_VISUALS: Record<string, CreatureVisual> = {
  "songbird": {
    view: { x0: -4, x1: 14, y0: -6, y1: 8 },
    silhouette: [[-3.2, -1.2], [-1.6, -3.4], [1.6, -4.6], [5.2, -4.4], [7.8, -2.6], [9.0, -0.4], [9.6, 1.8], [10.4, 2.4], [12.8, 2.6], [10.6, 3.0], [9.8, 3.6], [9.2, 4.6], [7.6, 5.2], [6.0, 4.6], [4.2, 2.8], [2.0, 2.0], [-0.2, 1.6], [-3.6, 1.4], [-3.4, 0.2]],
    parts: [{ shade: 0.2, pts: [[8.6, 4.0], [8.95, 4.3], [9.2, 4.0], [8.95, 3.7]] }],
    tubes: [
      { tube: "bronchusL", path: [[3.4, -1.9], [4.4, -1.4]], label: "Left bronchus", inView: false },
      { tube: "bronchusR", path: [[3.4, -0.9], [4.4, -1.2]], label: "Right bronchus", inView: false },
      { tube: "trachea", path: [[4.4, -1.3], [5.6, -0.4], [6.8, 0.8], [7.8, 1.9], [8.6, 2.5]], label: "Trachea" },
      { tube: "beak", path: [[9.4, 2.75], [10.6, 2.75], [12.4, 2.7]], label: "Beak" },
    ],
    tubeScale: 1.6,
    sources: [{ key: "left", at: [4.2, -1.6], label: "Left voice" }, { key: "right", at: [4.2, -1.0], label: "Right voice" }],
  },
  "seal": {
    view: { x0: -60, x1: 60, y0: -30, y1: 30 },
    silhouette: [[-58, -2], [-50, -8], [-30, -14], [-5, -16], [20, -14], [38, -10], [48, -6], [55, -3], [58, 1], [56, 5], [50, 8], [42, 10], [30, 11], [10, 13], [-15, 13], [-35, 10], [-50, 6], [-56, 3], [-62, 8], [-64, 4], [-60, 0], [-64, -4], [-62, -8]],
    parts: [{ shade: 0.15, pts: [[47, 4], [48.5, 5], [50, 4], [48.5, 3]] }],
    tubes: [
      { tube: "trachea", path: [[8, -4], [20, -3], [30, -2]], label: "Trachea" },
      { tube: "pharynx", path: [[30, -2], [38, 0], [44, 1.5]], label: "Pharynx" },
      { tube: "oral", path: [[44, 1.5], [50, 0.5], [57, -0.5]], label: "Mouth" },
    ],
    tubeScale: 1.3,
    sources: [{ key: "glottis", at: [30, -2], label: "Larynx" }],
  },
  "alligator": {
    view: { x0: -120, x1: 120, y0: -40, y1: 40 },
    silhouette: [[-118, 2], [-90, -4], [-60, -8], [-30, -14], [0, -16], [30, -14], [50, -10], [70, -8], [95, -5], [114, -3], [118, 0], [115, 3], [98, 5], [75, 7], [52, 9], [40, 12], [20, 14], [-10, 14], [-40, 11], [-70, 7], [-100, 4]],
    parts: [{ shade: 0.15, pts: [[54, 9], [56, 11], [58, 9], [56, 8]] }],
    tubes: [
      { tube: "trachea", path: [[0, -6], [18, -5], [36, -4]], label: "Trachea" },
      { tube: "pharynx", path: [[36, -4], [46, -2], [54, 0]], label: "Pharynx" },
      { tube: "oral", path: [[54, 0], [80, -1], [112, -1]], label: "Mouth" },
      { tube: "nasal", path: [[54, 2], [80, 4], [113, 2]], label: "Nostrils" },
    ],
    tubeScale: 1.05,
    sources: [{ key: "glottis", at: [36, -4], label: "Larynx" }],
  },
  "elephant": {
    view: { x0: -170, x1: 170, y0: -170, y1: 130 },
    silhouette: [[-160, 20], [-150, 70], [-110, 105], [-40, 118], [30, 115], [70, 100], [95, 112], [120, 108], [140, 85], [148, 55], [146, 25], [140, 5], [132, -20], [128, -60], [122, -110], [118, -150], [108, -152], [112, -100], [110, -50], [104, -20], [96, -5], [80, 0], [70, -10], [66, -90], [40, -95], [44, -30], [-90, -30], [-95, -95], [-125, -95], [-122, -10], [-150, -5]],
    parts: [{ shade: 0.15, pts: [[118, 62], [122, 66], [126, 62], [122, 58]] }, { shade: 0.62, pts: [[70, 100], [40, 60], [55, 25], [85, 40], [95, 80]] }],
    tubes: [
      { tube: "trachea", path: [[40, 20], [70, 30], [95, 38]], label: "Trachea" },
      { tube: "pharynx", path: [[95, 38], [112, 50], [124, 58]], label: "Pharynx" },
      { tube: "oral", path: [[124, 58], [130, 42], [134, 30]], label: "Mouth" },
      { tube: "trunk", path: [[136, 70], [142, 40], [136, 0], [128, -50], [120, -100], [113, -148]], label: "Trunk" },
    ],
    tubeScale: 3,
    sources: [{ key: "glottis", at: [95, 38], label: "Vocal folds" }, { key: "lips", at: [136, 70], label: "Trunk valve" }],
  },
  "cat": {
    view: { x0: -22, x1: 22, y0: -14, y1: 16 },
    silhouette: [[-18, -10], [-15, -5], [-10, -1], [-4, 1], [1, 3], [3, 9], [4.5, 13.5], [6.5, 10], [8.5, 10.2], [10.5, 13.2], [11.5, 8.8], [13.5, 6.2], [16.5, 4.4], [17.2, 2.5], [15.6, 0.8], [12.4, 0.2], [10.2, -2.5], [9.5, -9.5], [6.2, -10], [5.8, -5], [-4, -8], [-13, -10.5], [-18.5, -7], [-19.5, -3], [-17.5, -2.5], [-16.5, -6.5]],
    parts: [{ shade: 0.2, pts: [[11.6, 5.2], [12.5, 5.9], [13.3, 5.1], [12.4, 4.5]] }],
    tubes: [
      { tube: "trachea", path: [[0.5, -3.2], [4.8, -1.0], [8.2, 1.2]], label: "Trachea" },
      { tube: "pharynx", path: [[8.2, 1.2], [10.2, 2.4], [11.8, 3.1]], label: "Pharynx" },
      { tube: "oral", path: [[11.8, 3.1], [14.2, 2.5], [16.3, 1.9]], label: "Mouth" },
      { tube: "nasal", path: [[11.8, 3.6], [14.3, 4.1], [16.6, 3.6]], label: "Nose", inView: false },
    ],
    tubeScale: 1.4,
    sources: [{ key: "glottis", at: [8.2, 1.2], label: "Larynx" }],
  },
  "bullfrog": {
    view: { x0: -18, x1: 18, y0: -13, y1: 13 },
    silhouette: [[-14, -7], [-11, -3], [-6, 1.5], [0, 4.5], [5.5, 6.8], [7.8, 8.6], [10.2, 7.6], [13.5, 5.2], [15.2, 2.8], [14.6, 0.6], [12.8, -1.2], [12.2, -5.8], [9.2, -9.2], [4.5, -9.6], [1.2, -7.2], [-4.5, -8.2], [-11.5, -9.2]],
    parts: [
      { shade: 0.2, pts: [[7.8, 6.8], [8.9, 7.8], [9.9, 6.8], [8.9, 5.9]] },
      { shade: 0.58, pts: [[4.5, 4.2], [5.8, 5.4], [7.0, 4.2], [5.8, 3.0]] },
    ],
    tubes: [
      { tube: "trachea", path: [[-2.5, -1.2], [1.2, 0.2], [4.2, 1.2]], label: "Trachea" },
      { tube: "pharynx", path: [[4.2, 1.2], [6.8, 2.0], [9.0, 2.4]], label: "Pharynx", inView: false },
      { tube: "oral", path: [[9.0, 2.4], [11.8, 2.2], [14.4, 1.7]], label: "Mouth" },
      { tube: "sac", path: [[8.2, 1.2], [9.8, -2.2], [8.8, -6.2], [5.6, -7.4], [3.2, -5.2]], label: "Vocal sac" },
    ],
    tubeScale: 1.05,
    sources: [{ key: "glottis", at: [4.2, 1.2], label: "Vocal cords" }],
  },
  "elk": {
    view: { x0: -62, x1: 66, y0: -48, y1: 56 },
    silhouette: [[-48, -38], [-38, -18], [-24, -2], [-10, 12], [-2, 24], [4, 30], [10, 29], [18, 27], [30, 23], [44, 18], [52, 13], [50, 7], [38, 6], [24, 7], [14, 3], [8, -10], [2, -26], [-6, -40]],
    parts: [
      { shade: 0.2, pts: [[22, 21], [24.5, 23], [27, 21], [24.5, 19.5]] },
      { shade: 0.65, pts: [[2, 30], [-10, 42], [-26, 48], [-18, 43], [-6, 36], [-12, 47], [-4, 40], [4, 49], [6, 38], [14, 45], [9, 34], [6, 29]] },
    ],
    tubes: [
      { tube: "trachea", path: [[-18, -24], [-6, -8], [4, 6]], label: "Trachea" },
      { tube: "pharynx", path: [[4, 6], [11, 13], [18, 16]], label: "Pharynx" },
      { tube: "oral", path: [[18, 16], [33, 14], [49, 11]], label: "Mouth" },
      { tube: "nasal", path: [[18, 18], [34, 19], [50, 15]], label: "Nostrils", inView: false },
    ],
    tubeScale: 1.6,
    sources: [
      { key: "glottis", at: [4, 6], label: "Vocal folds" },
      { key: "whistle", at: [20, 15.5], label: "Palatal whistle" },
    ],
  },
  "whale": {
    view: { x0: -155, x1: 155, y0: -65, y1: 68 },
    silhouette: [[-145, 2], [-132, 10], [-95, 18], [-40, 28], [25, 38], [95, 44], [136, 42], [145, 30], [146, 10], [142, -6], [124, -10], [72, -11], [48, -14], [118, -18], [116, -24], [44, -22], [12, -26], [-45, -24], [-100, -14], [-134, -6], [-148, -16], [-144, -2]],
    parts: [{ shade: 0.2, pts: [[38, -6], [42, -3], [46, -6], [42, -9]] }],
    tubes: [
      { tube: "spermaceti", path: [[134, 30], [92, 31], [48, 28], [18, 24]], label: "Spermaceti sac" },
      { tube: "junk", path: [[130, 18], [96, 12], [60, 8], [140, 4]], label: "Junk lens" },
    ],
    tubeScale: 1.9,
    sources: [{ key: "phonic", at: [134, 30], label: "Phonic lips" }],
  },
};
