# Summary
Creating an automatic plant wall in a single month, with an STM32 written with Rust, an HMI, modular plant holders, and many, many valves. 

I am not great at keeping indoor plants consistently fed with water and light, but always overfeed or underfeed them.
I also love having plants inside the house, but they take up a lot of space, and it's hard to find ways to get the lighting correct.

Solution?
A plant wall that autofeeds, and control it's own lights.
HMI to make feeding system automated
A wall mounted system makes the space usage small.
Modular plant solution to make life easy

# Scope
This project is meant to be a sprint during the month of November. 
Expectations:
- Electrical system fully functioning
- Plant pods functioning, at least 3 created
- Plant rows functioning, at least 2 created
- Watering control loop fully functioning
- Lighting set to On/Off, individual plant control not in scope
- HMI functioning to select water amount
- Fault LED functioning for low fluid sensor

# Planning
## Main Plant Wall System Requirements
- Limited user inputs needed
  - Plant the plant in the Plant Pod, connect the hygrometer wire
  - Plant Pod simple to move from one Plant Row to another
  - Select how much water, and, light(TBD), feed time
  - Fill the water reservoir when LED flashes for it
  - Empty the drip tray if needed
- Safety: if the power goes out, water doesn't flow
- Prove out with 2 rows, 3 plant pods
## Future Requirements:
- Visual: I want it to physically look good
- Lights per pod with proper control loops and light guiding
- 5 plants per row
- Water tank to feed for minimum of 1 week with full fill


# Build

