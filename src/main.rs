use std::os::unix::fs::DirEntryExt;
//use std::sync::mpsc::{channel, Sender, Receiver};
use std::sync::{Arc, Mutex, Condvar, RwLock};
use std::thread;
use std::thread::sleep;
use crossbeam_channel::{unbounded, Receiver, select, bounded, Sender};
use std::time::Duration;
use std::collections::HashMap;
use rand::Rng;


enum ElevatorCommand {
    MoveTo(u8), // Bewege zu Ebene x
    OpenDoor,
    CloseDoor,
}

enum ControlCommand {
    Request { floor: u8, direction: Direction }
}

enum FloorCommand {
    Request { floor: u8, direction: Direction }
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Up,
    Down,
}

enum ElevatorState {
    IdleAtFloor(u8),      // Steht in einer Ebene, z. B. "IdleAtFloor(2)"
    Moving(u8, u8),       // Fährt von Ebene x zu Ebene y, z. B. "Moving(1, 3)"
    StoppedAtFloor(u8),
}

enum ElevatorStatus {
    DoorOpened(usize, u8), // Fahrstuhl-ID, Ebene
    DoorClosed(usize, u8),
    ArrivedAtFloor(usize, u8),
    TaskCompleted(usize), // Aufgabe erledigt, optional mit Zusatzinfo
}

enum DoorState {
    Closed,      // Tür ist geschlossen
    Opening,     // Tür öffnet sich
    Open,        // Tür ist offen
    Closing,     // Tür schließt sich
}

enum PassengerState {
    IdleAtFloor(u8),       // Wartet in einer Ebene
    EnteringElevator,      // Betritt den Fahrstuhl
    InElevator(u8),        // Ist im Fahrstuhl mit Ziel Ebene x
    ExitingElevator,       // Verlässt den Fahrstuhl
}

struct ControlSystem {
    elevators: Vec<Sender<ElevatorCommand>>,
    command_rx: Receiver<ControlCommand>,
    status_rx: Receiver<ElevatorStatus>, // Receiver für Statusupdates
}

impl ControlSystem {
    fn new(
        elevators: Vec<Sender<ElevatorCommand>>,
        command_rx: Receiver<ControlCommand>,
        status_rx: Receiver<ElevatorStatus>,
    ) -> Self {

        let control_system = Self {
            elevators,
            command_rx,
            status_rx,
        };

        let command_rx_clone = control_system.command_rx.clone();
        let status_rx_clone = control_system.status_rx.clone();
        let elevators_clone = control_system.elevators.clone();

        // ControlSystem-Thread starten
        thread::spawn(move || {
            ControlSystem::run(elevators_clone, command_rx_clone, status_rx_clone);
        });

        control_system
    }

    fn run(
        elevators: Vec<Sender<ElevatorCommand>>,
        command_rx: Receiver<ControlCommand>,
        status_rx: Receiver<ElevatorStatus>,
    ) {
        loop {
            select! {
                recv(command_rx) -> command => {
                    if let Ok(command) = command {
                        match command {
                            ControlCommand::Request { floor, direction } => {
                                println!("Control System: Received request from floor {} going {:?}", floor, direction);
                                let best_elevator = 0; // Einfache Logik: immer der erste Fahrstuhl
                                println!("Control System: Assigning Elevator {} to floor {}", best_elevator, floor);
                                elevators[best_elevator]
                                    .send(ElevatorCommand::MoveTo(floor))
                                    .unwrap();
                            }
                        }
                    }
                }
                recv(status_rx) -> status => {
                    if let Ok(status) = status {
                        match status {
                            ElevatorStatus::ArrivedAtFloor(id, floor) => {
                                println!("Elevator {} arrived at floor {}", id, floor);
                            }
                            ElevatorStatus::DoorOpened(id, floor) => {
                                println!("Elevator {} opened door at floor {}", id, floor);
                            }
                            ElevatorStatus::DoorClosed(id, floor) => {
                                println!("Elevator {} closed door at floor {}", id, floor);
                            }
                            ElevatorStatus::TaskCompleted(id) => {
                                println!("Elevator {} completed a task", id);
                            }
                        }
                    }
                }
            }
        }
    }
}


struct Elevator {
    id: usize,
    current_floor: u8,
    state: ElevatorState,
    door: Door,
    status_tx: Sender<ElevatorStatus>, // Sender für Statusupdates
    passenger_count: usize,
    elevator_floor_transmitter: Arc<RwLock<Vec<Sender<ElevatorArrived>>>>,
    elevator_to_passenger_transmitter: Arc<RwLock<Vec<Sender<ElevatorToPassenger>>>>,
    passenger_to_elevator_receiver: Arc<RwLock<Vec<Receiver<PassengerToElevator>>>>
}

impl Elevator {
    fn new(
        id: usize,
        rx: Receiver<ElevatorCommand>,
        status_tx: Sender<ElevatorStatus>,
    ) -> Arc<Mutex<Self>> {
        let elevator = Arc::new(Mutex::new(Self {
            id,
            current_floor: 0,
            state: ElevatorState::IdleAtFloor(0),
            door: Door::new(),
            status_tx,
            passenger_count: 0,
            elevator_floor_transmitter,
            elevator_to_passenger_transmitter,
            passenger_to_elevator_receiver
        }));

        let elevator_clone = Arc::clone(&elevator);

        // Elevator-Thread starten
        thread::spawn(move || {
            Self::run(elevator_clone, rx);
        });

        elevator
    }

    fn run(elevator: Arc<Mutex<Self>>, rx: Receiver<ElevatorCommand>) {
        loop {
            select! {
                recv(rx) -> command => {
                    if let Ok(command) = command {
                        // Nur während der Verarbeitung sperren
                        {
                            let mut elevator = elevator.lock().unwrap();
                            match command {
                                ElevatorCommand::MoveTo(floor) => {
                                    println!("Elevator {}: Received move request to floor {}", elevator.id, floor);
                                    elevator.move_to(floor);
                                    elevator.status_tx
                                        .send(ElevatorStatus::ArrivedAtFloor(elevator.id, floor))
                                        .unwrap();
                                }
                                ElevatorCommand::OpenDoor => {
                                    println!("Elevator {}: Received open door command", elevator.id);
                                    elevator.open_door();
                                    elevator.status_tx
                                        .send(ElevatorStatus::DoorOpened(elevator.id, elevator.current_floor))
                                        .unwrap();
                                }
                                ElevatorCommand::CloseDoor => {
                                    println!("Elevator {}: Received close door command", elevator.id);
                                    elevator.close_door();
                                    elevator.status_tx
                                        .send(ElevatorStatus::DoorClosed(elevator.id, elevator.current_floor))
                                        .unwrap();
                                }
                            }
                        } // `Mutex` wird hier automatisch freigegeben
                    }
                }
                default(Duration::from_millis(10000)) => {
                    println!("Waiting for a new command...");
                }
            }
        }
    }

    fn move_to(&mut self, target_floor: u8) {
        if (self.current_floor == 0 && target_floor < 0) || (self.current_floor == 3 && target_floor > 3) {
            println!("Elevator {}: Invalid move requested! Cannot move beyond floor limits.", self.id);
            return;
        }
        if let DoorState::Open = self.door.state {
            println!("Elevator {}: Cannot move while door is open!", self.id);
        } else {
            println!("Elevator {} moving from floor {} to floor {}", self.id, self.current_floor, target_floor);
            self.state = ElevatorState::Moving(self.current_floor, target_floor);
            self.current_floor = target_floor;
            println!("Elevator {}: Arrived at floor {}", self.id, target_floor);
            self.state = ElevatorState::IdleAtFloor(target_floor);
            self.elevator_floor_transmitter.read().unwrap().get(self.current_floor as usize)
                .unwrap().send(ElevatorArrived::Elevator(self.id as u8)).unwrap();

        }
    }

    fn open_door(&mut self) {
        self.door.open();
        self.state = ElevatorState::StoppedAtFloor(self.current_floor);
    }


    fn close_door(&mut self) {
            self.door.close();
    }


}

struct Door {
    state: DoorState,
}

impl Door {
    fn new() -> Self {
        Self {
            state: DoorState::Closed,
        }
    }

    fn open(&mut self) {
        if let DoorState::Closed = self.state {
            self.state = DoorState::Opening;
            println!("Opening the door...");
            thread::sleep(Duration::from_secs(1)); // Warte 1 Sekunde
            self.state = DoorState::Open;
            println!("Door is now open.");
        }
    }

    fn close(&mut self) {
        if let DoorState::Open = self.state {
            self.state = DoorState::Closing;
            println!("Closing the door...");
            thread::sleep(Duration::from_secs(1)); // Warte 1 Sekunde
            self.state = DoorState::Closed;
            println!("Door is now closed.");
        }
    }
}

struct Floor {
    id: u8,
    up_request: bool,
    down_request: bool,
    control_tx: Sender<ControlCommand>,
}

impl Floor {
    fn new(id: u8, control_tx: Sender<ControlCommand>, floor_rx: Receiver<FloorCommand>) {
        thread::spawn(move || {
            while let Ok(FloorCommand::Request { floor: _, direction }) = floor_rx.recv() { 
                control_tx
                    .send(ControlCommand::Request {
                        floor: id,
                        direction, 
                    })
                    .unwrap();
            }
        });
    }
}



fn main() {
    let floors = 4;
    let elevators = 3;
    let passengers = 2;

    let mut floor_channels: HashMap<u8, Sender<FloorCommand>> = HashMap::new();
    let mut elevator_senders = Vec::new();
    let (control_tx, control_rx) = unbounded();
    let (status_tx, status_rx) = unbounded();

    // Fahrstühle initialisieren
    for id in 0..elevators {
        let (elevator_tx, elevator_rx) = unbounded();
        elevator_senders.push(elevator_tx);
        Elevator::new(id, elevator_rx, status_tx.clone());
    }

    // Etagen initialisieren
    for i in 0..floors {
        let (floor_tx, floor_rx) = unbounded();
        floor_channels.insert(i, floor_tx);
        Floor::new(i, control_tx.clone(), floor_rx);
    }

    // Control System initialisieren
    let control_system = ControlSystem::new(elevator_senders, control_rx, status_rx);

    // Passagiere initialisieren
    for i in 0..passengers {
        
    }
    floor_channels.get(&0).unwrap().send(FloorCommand::Request { floor: 0, direction: Direction::Up }).unwrap();

    // Beispieleingaben
    // control_tx
    //     .send(ControlCommand::Request {
    //         floor: 2,
    //         direction: Direction::Up,
    //     })
    //     .unwrap();

    // Simulieren von Anfragen
    thread::sleep(Duration::from_secs(10));
}
