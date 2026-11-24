/** ACE-Step reads only the first 256 tokens of a training prompt; past about 850 characters (keyword,
 * caption, Instruments and Mood together) the API cuts the training text so bpm/key still fit. Keep in step
 * with MAX_TRAIN_CAPTION_CHARS in apps/api/src/routes/lora_builds.rs. */
export const TRAIN_CAPTION_CHARS = 850

export const KEY_PLACEHOLDER = 'A minor, C# major, Am'
