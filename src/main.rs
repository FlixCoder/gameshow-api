use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use actix_files::NamedFile;
use serde::{Serialize, Deserialize};
use dotenv::dotenv;
use tokio::sync::RwLock;
use std::sync::atomic::{Ordering, AtomicUsize};
use rand::seq::SliceRandom;
use std::fs;
use std::path::Path;
use std::env;


//fallback standards in case the ENV variable does not exist
const QUESTIONS_FILE:&str = "./Questions/questions-example.json"; //path to questions file
const INITIAL_MONEY:i64 = 500; //initial amount of money every player owns
const INITIAL_JOKERS:usize = 3; //number of inital jokers every player gets
const NORMAL_Q_MONEY:i64 = 500; //money to get when answering a normal question correctly
const ESTIMATION_Q_MONEY:i64 = 1000; //money to get when winning a estimation question

//struct for player data
#[derive(Serialize, Deserialize, Clone)]
struct PlayerData
{
    name: String,
    jokers: usize,
    money: i64,
    //could also use Option<>, but easier for frontend to handle without
    money_bet: i64,
    vs_player: String,
    answer: usize,
}

//different gameshow question types
#[derive(Serialize, Deserialize, Copy, Clone, PartialEq)]
enum QuestionType
{
    NormalQuestion,
    BettingQuestion,
    EstimationQuestion,
    VersusQuestion,
}

//struct for question data
#[derive(Serialize, Deserialize, Clone)]
struct Question
{
    question_type: QuestionType,
    category: String,
    question: String,
    answers: Vec<String>,
    correct_answer: usize,
}


//structs for events
#[derive(Serialize, Deserialize, Clone)]
struct EventBeginNormalQAnswering
{
    question_type: QuestionType,
    current_question: usize,
    category: String,
    question: String,
    answers: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone)]
struct EventBeginBettingQBetting
{
    question_type: QuestionType,
    current_question: usize,
    category: String,
}
#[derive(Serialize, Deserialize, Clone)]
struct EventBeginBettingQAnswering
{
    question: String,
    answers: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone)]
struct EventBeginEstimationQAnswering
{
    question_type: QuestionType,
    current_question: usize,
    category: String,
    question: String,
}
#[derive(Serialize, Deserialize, Clone)]
struct EventBeginVersusQSelecting
{
    question_type: QuestionType,
    current_question: usize,
    category: String,
}
#[derive(Serialize, Deserialize, Clone)]
struct EventBeginVersusQAnswering
{
    question: String,
    answers: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone)]
struct EventShowResults
{
    correct_answer: usize,
    previous_player_data: Vec<PlayerData>,
    player_data: Vec<PlayerData>,
}
#[derive(Serialize, Deserialize, Clone)]
struct EventGameEnding
{
    player_data: Vec<PlayerData>,
}
//combining struct for events
#[derive(Serialize, Deserialize, Clone)]
enum EventType
{
    BeginNormalQAnswering(EventBeginNormalQAnswering),
    BeginBettingQBetting(EventBeginBettingQBetting),
    BeginBettingQAnswering(EventBeginBettingQAnswering),
    BeginEstimationQAnswering(EventBeginEstimationQAnswering),
    BeginVersusQSelecting(EventBeginVersusQSelecting),
    BeginVersusQAnswering(EventBeginVersusQAnswering),
    ShowResults(EventShowResults),
    GameEnding(EventGameEnding),
}
#[derive(Serialize, Deserialize, Clone)]
struct Event
{
    id: usize,
    event_name: String,
    event: EventType,
}

#[derive(Serialize, Deserialize, Copy, Clone, PartialEq)]
enum QuestionState
{ //the bool indicates if it is ready to transition to next state
    Results(bool),
    NormalQAnswering(bool),
    BettingQBetting(bool),
    BettingQAnswering(bool),
    EstimationQAnswering(bool),
    VersusQSelecting(bool),
    VersusQAnswering(bool),
    GameEnding,
}


//database of all shared data for the gameshow
//lock order to avoid deadlocks: current_question_state -> questions -> player_data -> game_events
struct GameshowData
{
    player_data: RwLock<Vec<PlayerData>>,
    questions: RwLock<Vec<Question>>,
    game_events: RwLock<Vec<Event>>,
    current_question: AtomicUsize,
    current_question_state: RwLock<QuestionState>,
}


//read questions from a JSON file and return them
async fn read_questions(filename: impl AsRef<Path>) -> std::io::Result<Vec<Question>>
{
    let json_string = fs::read_to_string(filename)?;
    let questions: Vec<Question> = serde_json::from_str(&json_string)?;
    Ok(questions)
}

//check if next question state is possible/initiated and transition
//(by preparing everything and adding an event)
async fn check_state_add_events(data: web::Data<GameshowData>)
{
    let mut question_state = data.current_question_state.write().await;
    match *question_state
    {
        QuestionState::Results(true) => { //transition to next question (different states for different questions)
            //gather necessary data
            let question_id = data.current_question.fetch_add(1, Ordering::Relaxed) + 1;
            let questions = data.questions.read().await;
            let num_questions = (*questions).len();
            if question_id > num_questions
            { //game ending
                let access = data.player_data.read().await;
                let player_data = (*access).clone();
                //create event
                let mut events = data.game_events.write().await;
                let mut event_id = 0;
                let last = (*events).last();
                if last.is_some()
                {
                    event_id = last.unwrap().id + 1;
                }
                let new_event = Event { id: event_id, event_name: String::from("GameEnding"),
                    event: EventType::GameEnding(EventGameEnding { player_data: player_data }) };
                (*events).push(new_event);
                //set new question state
                *question_state = QuestionState::GameEnding;
            }
            else
            { //next question
                let question_type = (*questions)[question_id - 1].question_type.clone();
                let category = (*questions)[question_id - 1].category.clone();
                let question = (*questions)[question_id - 1].question.clone();
                let answers = (*questions)[question_id - 1].answers.clone();
                //reset bets and question answers for all players
                let mut player_access = data.player_data.write().await;
                for player in (*player_access).iter_mut()
                { //change zeros to None when using Options
                    player.money_bet = 0;
                    player.vs_player = "".to_owned();
                    player.answer = 0;
                }
                //depending on question type begin different question-specific event
                let mut events = data.game_events.write().await;
                let mut event_id = 0;
                let last = (*events).last();
                if last.is_some()
                {
                    event_id = last.unwrap().id + 1;
                }
                match question_type
                {
                    QuestionType::NormalQuestion => {
                        let event_data = EventBeginNormalQAnswering { question_type: question_type, current_question: question_id,
                            category: category, question: question, answers: answers };
                        let new_event = Event { id: event_id, event_name: String::from("BeginNormalQAnswering"),
                            event: EventType::BeginNormalQAnswering(event_data) };
                        (*events).push(new_event);
                        //set new question state
                        *question_state = QuestionState::NormalQAnswering(false);
                    },
                    QuestionType::BettingQuestion => {
                        let event_data = EventBeginBettingQBetting { question_type: question_type, current_question: question_id, category: category };
                        let new_event = Event { id: event_id, event_name: String::from("BeginBettingQBetting"),
                            event: EventType::BeginBettingQBetting(event_data) };
                        (*events).push(new_event);
                        //set new question state
                        *question_state = QuestionState::BettingQBetting(false);
                    },
                    QuestionType::EstimationQuestion => {
                        let event_data = EventBeginEstimationQAnswering { question_type: question_type, current_question: question_id, category: category,
                            question: question };
                        let new_event = Event { id: event_id, event_name: String::from("BeginEstimationQAnswering"),
                            event: EventType::BeginEstimationQAnswering(event_data) };
                        (*events).push(new_event);
                        //set new question state
                        *question_state = QuestionState::EstimationQAnswering(false);
                    },
                    QuestionType::VersusQuestion => {
                        let event_data = EventBeginVersusQSelecting { question_type: question_type, current_question: question_id, category: category };
                        let new_event = Event { id: event_id, event_name: String::from("BeginVersusQSelecting"),
                            event: EventType::BeginVersusQSelecting(event_data) };
                        (*events).push(new_event);
                        //set new question state
                        *question_state = QuestionState::VersusQSelecting(false);
                    },
                }
                
                
            }
        },
        QuestionState::BettingQBetting(true) => { //transition to answering state
            //gather necessary data
            let question_id = data.current_question.load(Ordering::Relaxed);
            let questions = data.questions.read().await;
            let question = (*questions)[question_id - 1].question.clone();
            let answers = (*questions)[question_id - 1].answers.clone();
            //create event
            let mut events = data.game_events.write().await;
            let mut event_id = 0;
            let last = (*events).last();
            if last.is_some()
            {
                event_id = last.unwrap().id + 1;
            }
            let event_data = EventBeginBettingQAnswering { question: question, answers: answers };
            let new_event = Event { id: event_id, event_name: String::from("BeginBettingQAnswering"),
                event: EventType::BeginBettingQAnswering(event_data) };
            (*events).push(new_event);
            //set new question state
            *question_state = QuestionState::BettingQAnswering(false);
        },
        QuestionState::VersusQSelecting(true) => { //transition to answering state
            //gather necessary data
            let question_id = data.current_question.load(Ordering::Relaxed);
            let questions = data.questions.read().await;
            let question = (*questions)[question_id - 1].question.clone();
            let answers = (*questions)[question_id - 1].answers.clone();
            //create event
            let mut events = data.game_events.write().await;
            let mut event_id = 0;
            let last = (*events).last();
            if last.is_some()
            {
                event_id = last.unwrap().id + 1;
            }
            let event_data = EventBeginVersusQAnswering { question: question, answers: answers };
            let new_event = Event { id: event_id, event_name: String::from("BeginVersusQAnswering"),
                event: EventType::BeginVersusQAnswering(event_data) };
            (*events).push(new_event);
            //set new question state
            *question_state = QuestionState::VersusQAnswering(false);
        },
        QuestionState::NormalQAnswering(true) => { //transition to results state
            //gather necessary data
            let question_id = data.current_question.load(Ordering::Relaxed);
            let questions = data.questions.read().await;
            let correct_answer = (*questions)[question_id - 1].correct_answer;
            //compute the new money of each player
            let mut access = data.player_data.write().await;
            let previous_player_data = (*access).clone();
            for player in (*access).iter_mut()
            {
                if player.answer == correct_answer
                {
                    let normal_q_money = env::var("NORMAL_Q_MONEY").unwrap_or_default().parse().unwrap_or(NORMAL_Q_MONEY);
                    player.money += normal_q_money;
                }
            }
            let player_data = (*access).clone();
            //create event
            let mut events = data.game_events.write().await;
            let mut event_id = 0;
            let last = (*events).last();
            if last.is_some()
            {
                event_id = last.unwrap().id + 1;
            }
            let event_data = EventShowResults { correct_answer: correct_answer, previous_player_data: previous_player_data, player_data: player_data };
            let new_event = Event { id: event_id, event_name: String::from("ShowResults"),
                event: EventType::ShowResults(event_data) };
            (*events).push(new_event);
            //set new question state
            *question_state = QuestionState::Results(false);
        },
        QuestionState::BettingQAnswering(true) => { //transition to results state
            //gather necessary data
            let question_id = data.current_question.load(Ordering::Relaxed);
            let questions = data.questions.read().await;
            let correct_answer = (*questions)[question_id - 1].correct_answer;
            //compute the new money of each player
            let mut access = data.player_data.write().await;
            let previous_player_data = (*access).clone();
            for player in (*access).iter_mut()
            {
                if player.answer == correct_answer
                {
                    player.money += player.money_bet;
                }
                else
                {
                    player.money -= player.money_bet;
                    //if player has no money, give 1€ to allow continuing the game
                    if player.money == 0
                    {
                        player.money = 1;
                    }
                }
            }
            let player_data = (*access).clone();
            //create event
            let mut events = data.game_events.write().await;
            let mut event_id = 0;
            let last = (*events).last();
            if last.is_some()
            {
                event_id = last.unwrap().id + 1;
            }
            let event_data = EventShowResults { correct_answer: correct_answer, previous_player_data: previous_player_data, player_data: player_data };
            let new_event = Event { id: event_id, event_name: String::from("ShowResults"),
                event: EventType::ShowResults(event_data) };
            (*events).push(new_event);
            //set new question state
            *question_state = QuestionState::Results(false);
        },
        QuestionState::EstimationQAnswering(true) => { //transition to results state
            //gather necessary data
            let question_id = data.current_question.load(Ordering::Relaxed);
            let questions = data.questions.read().await;
            let correct_answer = (*questions)[question_id - 1].correct_answer;
            //compute the new money of each player
            let mut closest_players = Vec::new();
            let mut min_dinstance = usize::MAX;
            let mut access = data.player_data.write().await;
            let previous_player_data = (*access).clone();
            for player in (*access).iter()
            {
                let diff = if player.answer >= correct_answer { player.answer - correct_answer } else { correct_answer - player.answer };
                if diff < min_dinstance
                {
                    min_dinstance = diff;
                    closest_players = vec![player.name.clone()];
                }
                else if diff == min_dinstance
                {
                    closest_players.push(player.name.clone());
                }
            }
            for player in (*access).iter_mut()
            {
                if closest_players.iter().any(|name| name == &player.name)
                {
                    let estimation_q_money = env::var("ESTIMATION_Q_MONEY").unwrap_or_default().parse().unwrap_or(ESTIMATION_Q_MONEY);
                    player.money += estimation_q_money;
                }
            }
            let player_data = (*access).clone();
            //create event
            let mut events = data.game_events.write().await;
            let mut event_id = 0;
            let last = (*events).last();
            if last.is_some()
            {
                event_id = last.unwrap().id + 1;
            }
            let event_data = EventShowResults { correct_answer: correct_answer, previous_player_data: previous_player_data, player_data: player_data };
            let new_event = Event { id: event_id, event_name: String::from("ShowResults"),
                event: EventType::ShowResults(event_data) };
            (*events).push(new_event);
            //set new question state
            *question_state = QuestionState::Results(false);
        },
        QuestionState::VersusQAnswering(true) => { //transition to results state
            //gather necessary data
            let question_id = data.current_question.load(Ordering::Relaxed);
            let questions = data.questions.read().await;
            let correct_answer = (*questions)[question_id - 1].correct_answer;
            //compute the new money of each player
            let mut access = data.player_data.write().await;
            let previous_player_data = (*access).clone();
            let num_players = (*access).len();
            let mut player_factors: Vec<f64> = vec![1.0; num_players];
            for i in 0 .. num_players
            {
                if (*access)[i].vs_player.is_empty() { continue; }
                for j in 0 .. num_players
                {
                    if (*access)[i].vs_player == (*access)[j].name
                    {
                        if (*access)[i].answer == correct_answer
                        {
                            //player_factors[i] *= 2.0;
                            player_factors[j] /= 2.0;
                        }
                        else
                        {
                            //player_factors[i] /= 2.0;
                            player_factors[j] *= 2.0;
                        }
                        break;
                    }
                }
            }
            for i in 0 .. num_players
            {
                (*access)[i].money = ((*access)[i].money as f64 * player_factors[i]) as i64;
                //if player has no money, give 1€ to allow continuing the game
                if (*access)[i].money == 0
                {
                    (*access)[i].money = 1;
                }
            }
            let player_data = (*access).clone();
            //create event
            let mut events = data.game_events.write().await;
            let mut event_id = 0;
            let last = (*events).last();
            if last.is_some()
            {
                event_id = last.unwrap().id + 1;
            }
            let event_data = EventShowResults { correct_answer: correct_answer, previous_player_data: previous_player_data, player_data: player_data };
            let new_event = Event { id: event_id, event_name: String::from("ShowResults"),
                event: EventType::ShowResults(event_data) };
            (*events).push(new_event);
            //set new question state
            *question_state = QuestionState::Results(false);
        },
        _ => {},
    }
}


//index site of API as information for me, which API interfaces are available. should not be visible not for users!
#[get("/api/")]
async fn index() -> impl Responder
{
    NamedFile::open("API-Overview.htm")
}

//join / register new player; struct for accepting the GET parameters
#[derive(Serialize, Deserialize)]
struct JoinPlayerData
{
    name: String,
}
#[get("/api/joinPlayer")]
async fn join_player(data: web::Data<GameshowData>, params: web::Query<JoinPlayerData>) -> impl Responder
{
    let trimmed_name = String::from(params.name.trim());
    if trimmed_name == ""
    {
        return HttpResponse::BadRequest().body("Empty name is not allowed!");
    }

    let mut access = data.player_data.write().await;
    if (*access).iter().all(|s| &s.name != &params.name)
    { //only append player if it is not contained already
        let initial_money = env::var("INITIAL_MONEY").unwrap_or_default().parse().unwrap_or(INITIAL_MONEY);
        let initial_jokers = env::var("INITIAL_JOKERS").unwrap_or_default().parse().unwrap_or(INITIAL_JOKERS);
        let new_player = PlayerData { name: trimmed_name.clone(), jokers: initial_jokers, money: initial_money,
            money_bet: 0, vs_player: "".to_owned(), answer: 0 };
        (*access).push(new_player);
    }

    HttpResponse::Ok().body(trimmed_name)
}

//list all registered players' data (also given answers leaked!)
#[get("/api/getPlayerData")]
async fn get_player_data(data: web::Data<GameshowData>) -> impl Responder
{
    let access = data.player_data.read().await;
    let player_data = (*access).clone();

    HttpResponse::Ok().json(player_data)
}

//accept the bets, but only when in betting question state
#[derive(Serialize, Deserialize)]
struct BetMoneyData
{
    name: String,
    money_bet: i64,
}
#[get("/api/betMoney")]
async fn bet_money(data: web::Data<GameshowData>, params: web::Query<BetMoneyData>) -> impl Responder
{
    //ensure current question state is betting, else return not acceptable
    {
        let question_state = data.current_question_state.read().await;
        if *question_state != QuestionState::BettingQBetting(false)
        {
            return HttpResponse::NotAcceptable().body("QuestionState is not Betting(false)!");
        }
    }
    
    //perform money betting
    {
        let mut name_found = false;
        let mut access = data.player_data.write().await;
        
        for player in (*access).iter_mut()
        {
            if player.name == params.name
            {
                if params.money_bet < 1 || player.money < params.money_bet
                {
                    return HttpResponse::BadRequest().body("money_bet is invalid (< 1 or > player money)!");
                }
                else
                { //set player's money_bet
                    player.money_bet = params.money_bet;
                    name_found = true;
                    break;
                }
            }
        }
        if !name_found
        {
            return HttpResponse::BadRequest().body("Player name was not found!");
        }
    }
    
    //check if all players have bet to indicate abilitiy to proceed
    let mut all_bet = true;
    {
        let access = data.player_data.read().await;
        for player in (*access).iter()
        {
            if player.money_bet < 1
            {
                all_bet = false;
                break;
            }
        }
    }
    if all_bet
    {
        let mut question_state = data.current_question_state.write().await;
        *question_state = QuestionState::BettingQBetting(true);
    }
    
    HttpResponse::Ok().finish()
}

//accept the versus selection, but only when in selecting question state
#[derive(Serialize, Deserialize)]
struct AttackPlayerData
{
    name: String,
    vs_player: String,
}
#[get("/api/attackPlayer")]
async fn attack_player(data: web::Data<GameshowData>, params: web::Query<AttackPlayerData>) -> impl Responder
{
    //ensure current question state is selecting, else return not acceptable
    {
        let question_state = data.current_question_state.read().await;
        if *question_state != QuestionState::VersusQSelecting(false)
        {
            return HttpResponse::NotAcceptable().body("QuestionState is not VersusQSelecting(false)!");
        }
    }
    
    //perform selecting
    {
        if params.name == params.vs_player
        {
            return HttpResponse::BadRequest().body("name and vs_player are equal!");
        }
        
        let mut access = data.player_data.write().await;
        if !(*access).iter().any(|player| &player.name == &params.name)
        {
            return HttpResponse::BadRequest().body("Player name was not found!");
        }
        if !(*access).iter().any(|player| &player.name == &params.vs_player)
        {
            return HttpResponse::BadRequest().body("Player vs_player was not found!");
        }
        
        for player in (*access).iter_mut()
        {
            if player.name == params.name
            { //set player's selection
                player.vs_player = params.vs_player.clone();
                break;
            }
        }
    }
    
    //check if all players have selected to indicate abilitiy to proceed
    let mut all_selected = true;
    {
        let access = data.player_data.read().await;
        for player in (*access).iter()
        {
            if player.vs_player == ""
            {
                all_selected = false;
                break;
            }
        }
    }
    if all_selected
    {
        let mut question_state = data.current_question_state.write().await;
        *question_state = QuestionState::VersusQSelecting(true);
    }
    
    HttpResponse::Ok().finish()
}

//accept the question answer
#[derive(Serialize, Deserialize)]
struct AnswerQuestionData
{
    name: String,
    answer: usize,
}
#[get("/api/answerQuestion")]
async fn answer_question(data: web::Data<GameshowData>, params: web::Query<AnswerQuestionData>) -> impl Responder
{
    //ensure current question state is answering, else return not acceptable
    {
        let question_state = data.current_question_state.read().await;
        if *question_state != QuestionState::NormalQAnswering(false) &&
            *question_state != QuestionState::BettingQAnswering(false) &&
            *question_state != QuestionState::EstimationQAnswering(false) &&
            *question_state != QuestionState::VersusQAnswering(false)
        {
            return HttpResponse::NotAcceptable().body("QuestionState is not *Answering(false)!");
        }
    }
    
    //perform answering
    {
        if params.answer < 1
        {
            return HttpResponse::BadRequest().body("answer is invalid (< 1)!");
        }
        
        let mut name_found = false;
        let mut access = data.player_data.write().await;
        for player in (*access).iter_mut()
        {
            if player.name == params.name
            { //set player's answer
                player.answer = params.answer;
                name_found = true;
                break;
            }
        }
        if !name_found
        {
            return HttpResponse::BadRequest().body("Player name was not found!");
        }
    }
    
    //check if all players have answered to indicate abilitiy to proceed
    let mut all_answered = true;
    {
        let access = data.player_data.read().await;
        for player in (*access).iter()
        {
            if player.answer < 1
            {
                all_answered = false;
                break;
            }
        }
    }
    if all_answered
    {
        let mut question_state = data.current_question_state.write().await;
        match *question_state
        {
            QuestionState::NormalQAnswering(_) => { *question_state = QuestionState::NormalQAnswering(true); },
            QuestionState::BettingQAnswering(_) => { *question_state = QuestionState::BettingQAnswering(true); },
            QuestionState::EstimationQAnswering(_) => { *question_state = QuestionState::EstimationQAnswering(true); },
            QuestionState::VersusQAnswering(_) => { *question_state = QuestionState::VersusQAnswering(true); },
            _ => {},
        }
    }
    
    HttpResponse::Ok().finish()
}

//get 50/50 joker for current question (only for betting questions!)
#[derive(Serialize, Deserialize)]
struct GetJokerData
{
    name: String,
}
#[get("/api/getJokerFiftyFifty")]
async fn get_joker_fifty_fifty(data: web::Data<GameshowData>, params: web::Query<GetJokerData>) -> impl Responder
{
    //ensure current question state is answering for normal or betting question, else return not acceptable
    {
        let question_state = data.current_question_state.read().await;
        if *question_state != QuestionState::NormalQAnswering(false) &&
            *question_state != QuestionState::BettingQAnswering(false)
        {
            return HttpResponse::NotAcceptable().body("QuestionState is not BettingQAnswering(false)!");
        }
    }
    
    //get wrong answers
    let wrong_answers: Vec<usize>;
    {
        let mut rng = rand::thread_rng();
        let current_question = data.current_question.load(Ordering::Relaxed);
        let questions = data.questions.read().await;
        let correct_answer = (*questions)[current_question - 1].correct_answer;
        let mut choose_from = vec![1, 2, 3, 4];
        choose_from.remove(correct_answer - 1); //removed by index
        wrong_answers = choose_from.choose_multiple(&mut rng, 2).copied().collect();
    }
    
    //send wrong answers
    let mut access = data.player_data.write().await;
    for player in (*access).iter_mut()
    {
        if player.name == params.name
        { //set player's answer
            if player.jokers < 1
            {
                return HttpResponse::NotAcceptable().body("No jokers available!");
            }
            else
            {
                player.jokers -= 1;
                return HttpResponse::Ok().json(wrong_answers);
            }
        }
    }
    
    HttpResponse::BadRequest().body("Player name was not found!")
}

//get current status and game commands
#[get("/api/getGameEvents")]
async fn get_game_events(data: web::Data<GameshowData>) -> impl Responder
{
    check_state_add_events(data.clone()).await;
    
    let access = data.game_events.read().await;
    let data = (*access).clone();
    
    HttpResponse::Ok().json(data)
}

//give a player money, minus value to remove money
#[derive(Serialize, Deserialize)]
struct GiveMoneyData
{
    name: String,
    money: i64,
}
#[post("/api/giveMoney")]
async fn give_money(data: web::Data<GameshowData>, params: web::Json<GiveMoneyData>) -> impl Responder
{
    let mut access = data.player_data.write().await;
    
    for player in (*access).iter_mut()
    {
        if player.name == params.name
        {
            player.money += params.money;
            return HttpResponse::Ok().json(GiveMoneyData {name: player.name.clone(), money: player.money});
        }
    }
    
    HttpResponse::BadRequest().body("Player name was not found!")
}

//set a player's number of available jokers
#[derive(Serialize, Deserialize)]
struct SetJokersData
{
    name: String,
    jokers: usize,
}
#[post("/api/setJokers")]
async fn set_jokers(data: web::Data<GameshowData>, params: web::Json<SetJokersData>) -> impl Responder
{
    let mut access = data.player_data.write().await;
    
    for player in (*access).iter_mut()
    {
        if player.name == params.name
        {
            player.jokers = params.jokers;
            return HttpResponse::Ok().json(SetJokersData {name: player.name.clone(), jokers: player.jokers});
        }
    }
    
    HttpResponse::BadRequest().body("Player name was not found!")
}

//kick a player
#[derive(Serialize, Deserialize)]
struct KickPlayerData
{
    name: String,
}
#[get("/api/kickPlayer")]
async fn kick_player(data: web::Data<GameshowData>, params: web::Query<KickPlayerData>) -> impl Responder
{
    let mut access = data.player_data.write().await;
    
    let len = (*access).len();
    (*access).retain(|player| player.name != params.name);
    if (*access).len() == len
    { //player was not found
        return HttpResponse::BadRequest().body("Player name was not found!");
    }
    
    HttpResponse::Ok().finish()
}

//activate next question, will fail if current question was not finished
#[get("/api/activateNextQuestion")]
async fn activate_next_question(data: web::Data<GameshowData>) -> impl Responder
{
    //check if game state is ready for next question
    let mut access = data.current_question_state.write().await;
    if let QuestionState::Results(_) = *access
    { //indicate possible transition to next question for automatic switch
        *access = QuestionState::Results(true);
        return HttpResponse::Ok().finish();
    }
    else
    {
        return HttpResponse::NotAcceptable().body("QuestionState is not Results! => Not ready for next question!");
    }
}

//force end of betting and activate question answering
#[get("/api/forceQuestionAnswering")]
async fn force_question_answering(data: web::Data<GameshowData>) -> impl Responder
{
    //ensure current question state is betting or selecting, else return not acceptable
    let mut question_state = data.current_question_state.write().await;
    match *question_state
    {
        QuestionState::BettingQBetting(false) => { *question_state = QuestionState::BettingQBetting(true); },
        QuestionState::VersusQSelecting(false) => { *question_state = QuestionState::VersusQSelecting(true); },
        _ => { return HttpResponse::NotAcceptable().body("QuestionState is not Betting(false) or Selecting(false)!"); },
    }
    HttpResponse::Ok().finish()
}

//force end of question answering and show results
#[get("/api/forceQuestionResults")]
async fn force_question_results(data: web::Data<GameshowData>) -> impl Responder
{
    //ensure current question state is answering, else return not acceptable
    let mut question_state = data.current_question_state.write().await;
    match *question_state
    {
        QuestionState::NormalQAnswering(false) => { *question_state = QuestionState::NormalQAnswering(true) },
        QuestionState::BettingQAnswering(false) => { *question_state = QuestionState::BettingQAnswering(true); },
        QuestionState::EstimationQAnswering(false) => { *question_state = QuestionState::EstimationQAnswering(true); },
        QuestionState::VersusQAnswering(false) => { *question_state = QuestionState::VersusQAnswering(true); },
        _ => { return HttpResponse::NotAcceptable().body("QuestionState is not *Answering(false)!"); },
    }
    HttpResponse::Ok().finish()
}

//set the next question (only possible, when currently in results state)
#[derive(Serialize, Deserialize)]
struct SetNextQuestionData
{
    number: usize,
}
#[get("/api/setNextQuestion")]
async fn set_next_question(data: web::Data<GameshowData>, params: web::Query<SetNextQuestionData>) -> impl Responder
{
    //ensure current question state is results or ended game, else return not acceptable; hold the lock until finished this time
    let mut question_state = data.current_question_state.write().await;
    if *question_state != QuestionState::Results(false) && *question_state != QuestionState::GameEnding
    {
        return HttpResponse::NotAcceptable().body("QuestionState is not Results(false) or GameEnding!");
    }
    
    let questions = data.questions.read().await;
    if params.number < 1 || params.number > (*questions).len()
    {
        return HttpResponse::BadRequest().body("Number is not a valid question ID (must be 1 - len(questions))!");
    }
    else
    {
        let question_id = data.current_question.swap(params.number - 1, Ordering::Relaxed);
        *question_state = QuestionState::Results(false);
        return HttpResponse::Ok().body(question_id.to_string());
    }
}

//load questions from a the given filename
#[derive(Serialize, Deserialize)]
struct LoadQuestions
{
    filename: String,
}
#[post("/api/loadQuestions")]
async fn load_questions(data: web::Data<GameshowData>, params: web::Json<LoadQuestions>) -> impl Responder
{
    //ensure current question state is results or ended game, else return not acceptable; hold the lock until finished this time
    let mut question_state = data.current_question_state.write().await;
    if *question_state != QuestionState::Results(false) && *question_state != QuestionState::GameEnding
    {
        return HttpResponse::NotAcceptable().body("QuestionState is not Results(false) or GameEnding!");
    }
    
    let new_questions = read_questions(String::from("./Questions/") + &params.filename).await;
    if new_questions.is_err()
    {
        return HttpResponse::BadRequest().body("Question file could not be loaded!");
    }
    else
    {
        let mut questions = data.questions.write().await;
        (*questions) = new_questions.ok().unwrap();
        data.current_question.store(0, Ordering::Relaxed);
        *question_state = QuestionState::Results(false);
        return HttpResponse::Ok().body((*questions).len().to_string());
    }
}



#[actix_web::main]
async fn main() -> std::io::Result<()>
{
    dotenv().ok();

    let questions_file = env::var("QUESTIONS_FILE").unwrap_or(String::from(QUESTIONS_FILE));
    let questions = read_questions(questions_file).await?;
    
    let data = web::Data::new(GameshowData {
        player_data: RwLock::new(Vec::new()),
        questions: RwLock::new(questions),
        game_events: RwLock::new(Vec::new()),
        current_question: AtomicUsize::new(0),
        current_question_state: RwLock::new(QuestionState::Results(false)),
    });

    HttpServer::new(move || {
        App::new()
            //shared data to store the gameshow state etc.
            .app_data(data.clone())

            //service the API sites/functions
            .service(index)
            .service(join_player)
            .service(get_player_data)
            .service(bet_money)
            .service(attack_player)
            .service(answer_question)
            .service(get_joker_fifty_fifty)
            .service(get_game_events)
            .service(give_money)
            .service(set_jokers)
            .service(kick_player)
            .service(activate_next_question)
            .service(force_question_answering)
            .service(force_question_results)
            .service(set_next_question)
            .service(load_questions)

            //publish the gameshow's frontend (static files)
            //(must be last, so that the specific handlers are served)
            .service(actix_files::Files::new("/", "./Gameshow").index_file("play.htm"))
            //.service(actix_files::Files::new("/", "./Gameshow").show_files_listing())
    })
    .bind("127.0.0.1:8000")?
    .run()
    .await
}

