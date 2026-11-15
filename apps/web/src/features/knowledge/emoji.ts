export type EmojiGroup = { id: string; label: string; emoji: string[] }

const group = (id: string, label: string, entries: string) => ({
  id,
  label,
  emoji: entries
    .trim()
    .split('\n')
    .map((line) => line.trim()),
})

export const EMOJI: EmojiGroup[] = [
  group(
    'docs',
    'Docs and work',
    `
📄 page document file doc
📝 memo note write draft
📌 pin pinned important
📍 location place pin
📎 clip attachment
📚 books library reading
📖 book open guide manual
📒 ledger notebook
📓 notebook journal
📋 clipboard checklist
🗂️ index dividers folders
📁 folder
🗃️ card box archive
🗄️ cabinet archive storage
📅 calendar date
🗓️ calendar plan schedule
📆 calendar tear
⏰ alarm clock deadline
⌛ hourglass time wait
✅ check done complete
☑️ checkbox task
✔️ tick
❌ cross no wrong
⚠️ warning caution
❗ exclamation important
❓ question faq help
💬 speech chat talk discussion
🗨️ comment
📣 megaphone announcement
📢 loudspeaker announce
🔔 bell notification
🏷️ tag label
🔖 bookmark
✏️ pencil edit
🖊️ pen sign
🖋️ fountain pen
📐 ruler triangle design
📏 ruler measure
✂️ scissors cut
🧾 receipt invoice
💼 briefcase business
🗳️ ballot vote
📨 incoming envelope inbox
📧 email mail
📮 postbox
📦 package box shipping release`,
  ),
  group(
    'tech',
    'Tech and tools',
    `
💻 laptop computer code
🖥️ desktop computer
⌨️ keyboard
🖱️ mouse
🖨️ printer
📱 phone mobile
🔌 plug integration
🔋 battery power
💾 disk save storage
💿 disc
🧮 abacus math calculate
🤖 robot bot agent ai
🧠 brain memory think
🛠️ tools build fix
🔧 wrench settings
🔨 hammer build
⚙️ gear settings config
🔩 bolt
🧰 toolbox
🪛 screwdriver
🧲 magnet
⛓️ chain link
🔗 link url
🧪 test tube experiment test
🔬 microscope research
🔭 telescope explore
📡 antenna satellite signal
🛰️ satellite
🧬 dna
🐛 bug issue
🪲 beetle bug
🗜️ clamp compress
💡 idea bulb light
🔦 flashlight search
🕹️ joystick game
📊 bar chart stats metrics
📈 chart up growth
📉 chart down decline
🧷 safety pin
🧱 brick infrastructure
🏗️ construction building wip
🛢️ oil drum database
☁️ cloud
🌐 globe web internet
🕸️ web network
🧯 extinguisher incident fire`,
  ),
  group(
    'security',
    'Security and access',
    `
🔒 lock secure private
🔓 unlocked open
🔐 lock key secure
🔑 key access credential
🗝️ old key secret
🛡️ shield security protect
🚨 siren alert incident
🚧 barrier construction blocked
🚫 prohibited forbidden
⛔ no entry stop
🆔 id identity
👤 person user account
👥 people users team
🕵️ detective investigate
🧿 eye amulet watch
👁️ eye view watch
🪪 id card identity
🏛️ institution policy legal
⚖️ scales law balance
📜 scroll policy rules history`,
  ),
  group(
    'ideas',
    'Ideas and goals',
    `
🚀 rocket launch ship
🎯 target goal aim okr
🏆 trophy win award
🥇 gold medal first
🏁 finish flag
🚩 red flag
⭐ star favorite
🌟 glowing star highlight
✨ sparkles new magic
🔥 fire hot urgent
⚡ lightning fast power
💎 gem value premium
🧭 compass direction strategy
🗺️ map roadmap plan
🧩 puzzle piece
🎨 palette design art
🎬 clapper video film
🎤 microphone talk podcast
🎧 headphones audio listen
🎵 music note
🎁 gift present
🎉 party celebrate launch
🎈 balloon celebrate
🧸 teddy bear
🪄 magic wand
🔮 crystal ball future predict
♻️ recycle reuse
🌀 cyclone loop
💫 dizzy
🌈 rainbow`,
  ),
  group(
    'people',
    'People and faces',
    `
😀 grinning happy
😄 smile happy
😎 cool sunglasses
🤓 nerd study
🤔 thinking question
🧐 monocle inspect review
😴 sleeping rest
🥳 party celebrate
🤯 mind blown
😬 grimace awkward
🙌 raised hands celebrate
👏 clap applause
👍 thumbs up approve
👋 wave hello welcome onboarding
🤝 handshake deal partner
🙏 thanks please
💪 strong muscle
✋ hand stop
👀 eyes look review
🧑‍💻 technologist developer engineer
🧑‍🔬 scientist research
🧑‍🏫 teacher training learn
🧑‍🎨 artist designer
🧑‍💼 office worker manager
🧑‍🚀 astronaut explore
🧙 mage wizard
🦸 superhero
🫶 heart hands care
❤️ heart love
💛 yellow heart
💚 green heart
💙 blue heart
💜 purple heart
🧡 orange heart`,
  ),
  group(
    'nature',
    'Nature and animals',
    `
🌱 seedling grow start
🌿 herb
🍀 clover luck
🌳 tree
🌲 evergreen tree
🌵 cactus
🌸 blossom spring
🌻 sunflower
🍁 maple leaf autumn
🍂 leaves fall
🌊 wave ocean water
🏔️ mountain
🌋 volcano
🏝️ island vacation
☀️ sun
🌙 moon night
⛅ cloud sun weather
❄️ snowflake winter freeze
🌍 earth globe world
🐱 cat
🐶 dog
🦊 fox
🐻 bear
🐼 panda
🐨 koala
🦁 lion
🐯 tiger
🐸 frog
🐵 monkey
🐧 penguin
🦉 owl wisdom
🦅 eagle
🐝 bee busy
🦋 butterfly change
🐢 turtle slow
🐙 octopus
🐳 whale
🦄 unicorn
🐉 dragon
🦀 crab rust`,
  ),
  group(
    'life',
    'Food, places and travel',
    `
☕ coffee break
🍵 tea
🍕 pizza
🍔 burger
🍣 sushi
🍎 apple
🍋 lemon
🥑 avocado
🍪 cookie
🎂 cake birthday
🍽️ dinner food
🏠 house home
🏡 home garden
🏢 office building company
🏭 factory production
🏥 hospital health
🏫 school education
🏦 bank finance
🏪 store shop
⛺ tent camp
✈️ airplane travel flight
🚗 car drive
🚲 bicycle
🚆 train
🚢 ship boat
⚓ anchor
🧳 luggage travel trip
💰 money bag budget
💵 dollar cash
💳 card payment billing
🛒 cart shopping
🏋️ weights fitness gym
🧘 yoga calm wellbeing
⚽ soccer sport
🎮 game controller
🎲 dice chance random
♟️ chess strategy
🧶 yarn
🪴 potted plant`,
  ),
  group(
    'symbols',
    'Symbols',
    `
🔴 red circle
🟠 orange circle
🟡 yellow circle
🟢 green circle
🔵 blue circle
🟣 purple circle
⚫ black circle
⚪ white circle
🟥 red square
🟧 orange square
🟨 yellow square
🟩 green square
🟦 blue square
🟪 purple square
🔶 orange diamond
🔷 blue diamond
➕ plus add
➖ minus remove
➡️ right arrow next
⬅️ left arrow back
🔁 repeat loop recurring
🔄 refresh sync update
🔀 shuffle
♾️ infinity
💯 hundred perfect
🆕 new
🆗 ok
🔝 top
ℹ️ information info
#️⃣ hash number
🔢 numbers
🔤 letters abc
💲 dollar sign price
©️ copyright
™️ trademark`,
  ),
]

export const symbol = (entry: string) => entry.slice(0, entry.indexOf(' '))
export const terms = (entry: string) => entry.slice(entry.indexOf(' ') + 1)

export function emojiMatches(query: string) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  return EMOJI.flatMap((g) => g.emoji).filter((entry) => {
    const own = terms(entry).split(' ')
    return words.every((word) => own.some((term) => term.startsWith(word)))
  })
}
