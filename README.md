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
## Requirements
### System: Plant Wall
#### Main Plant Wall System Requirements
- Limited user inputs needed
  - Plant the plant in the Plant Pod, connect the hygrometer wire
  - Plant Pod simple to move from one Plant Row to another
  - Select how much water, and, light(TBD), feed time
  - Fill the water reservoir when LED flashes for it
  - Empty the drip tray if needed
- Safety: if the power goes out, water doesn't flow
- Prove out with 2 rows, 3 plant pods
#### Future Requirements:
- Visual: I want it to physically look good
- Lights per pod with proper control loops and light guiding
- 5 plants per row
- Water tank to feed for minimum of 1 week with full fill

### Sub-System: Mechanical
#### Main Mechanical Subsystem Requirements:
- Use of common hardware for ease of repair
- Valves to be normally closed to limit power draw, or allow water though when power loss
- Easy refill option
- Pod install/uninstall to be quick and easy
#### Future Requirements
- Adjustable height for the plant row to accommodate different sized plants
- Look good

### Sub-System: Electrical
#### Main Electrical Subsystem Requirements:
- 12V power for valves
- 12V power for pump
- 3.3-12V power for Hygrometers
- 5-24V for flow meter
- HMI
  - Clock using 4 digit 7 segment display
  - LCD display for user interaction
  - D-pad or Analog stick for user control
  - 2x buttons for user control

#### Future Requirements

### Sub-System: Software
#### Main Software Subsystem Requirements:
- Built in RUST
- Control row lights based on timer for prove out project
- Control loop from Hygrometer:
  - Control pump for water flow
  - Control valves for individual plant pod control
  - Flow rate input for modeling and knowledge of water amount - removes need for valves at every row
  - Hygrometer input for soil humidity level
- HMI:
  - Settings are able to be saved
  - Select watering amount
  - Select lighting, in time
  - Current time shown
  - Selectable time of day to water

#### Future Requirements:
- Selectable profiles for the type of plant it is
- Control loop for lighting with sensor for lighting amount

# Build




### Creating a parameter file
Parameters to include:
- 

### Creating a datalog system to tune the parameters
- List of signals to trace?
- How to define? A2L?
- 

