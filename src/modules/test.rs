use core::BotCmdAuthLvl as Auth;
use core::*;
use std::mem;
use yaml_rust::Yaml;

pub fn mk() -> Module {
    mk_module("test")
        .on_load(Box::new(|_: &State| {
            trace!("Hello from the `test` module's `on_load` function!");
            Ok(())
        }))
        .command(
            "test-line-wrap",
            "",
            "Request a long message from the bot, to test its line-wrapping function.",
            Auth::Admin,
            Box::new(test_line_wrap),
            &[],
        )
        .command(
            "test-error-handling",
            "",
            "This command's handler function returns an error, to test the bot framework's \
             error-handling mechanism(s).",
            Auth::Admin,
            Box::new(test_error_handling),
            &[],
        )
        .command(
            "test-panic-catching",
            "",
            "This command's handler function panics, to test the bot framework's panic-catching \
             mechanism.",
            Auth::Admin,
            Box::new(test_panic_catching),
            &[],
        )
        .command(
            "test-stack-overflow",
            "",
            "This command's handler function allocates an enormous value on the stack, to test \
             how the bot framework handles stack overflow in command handler functions. \
             (Currently (October 2018), this simply makes the bot crash.)",
            Auth::Admin, // TODO: Use `Auth::Owner` once available.
            Box::new(test_stack_overflow),
            &[],
        )
        .end()
}

const LOREM_IPSUM_TEXT: &'static str =
    "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Integer et tincidunt nibh. Nullam \
     aliquet imperdiet cursus. Duis at turpis mollis, iaculis quam sed, efficitur arcu. Sed vel \
     massa sit amet magna efficitur hendrerit. Donec auctor auctor ligula nec semper. Nulla a \
     odio suscipit, suscipit velit in, ullamcorper velit. In bibendum pulvinar ipsum. Fusce \
     elementum maximus mattis. Donec sed mauris nec ante eleifend dapibus non faucibus massa. \
     Vivamus a auctor ligula. Cras hendrerit, velit sit amet sagittis placerat, elit elit feugiat \
     quam, vel aliquet ligula elit sit amet nibh. Fusce dignissim, orci vitae sodales ornare, \
     lacus risus facilisis sem, a imperdiet lectus massa at velit. Etiam sed magna congue, \
     pulvinar diam quis, facilisis risus. Sed semper, lectus vulputate luctus fermentum, quam \
     lacus consectetur arcu, ac mollis ipsum metus vel nunc. Ut posuere arcu enim, id dictum arcu \
     sagittis in. Mauris a lectus nec ligula eleifend rutrum. Class aptent taciti sociosqu ad \
     litora torquent per conubia massa nunc.";

fn test_line_wrap(_: HandlerContext, _: &Yaml) -> BotCmdResult {
    BotCmdResult::Ok(Reaction::Reply(LOREM_IPSUM_TEXT.into()))
}

fn test_error_handling(_: HandlerContext, _: &Yaml) -> BotCmdResult {
    BotCmdResult::BotErrMsg("An error for testing purposes.".into())
}

fn test_panic_catching(_: HandlerContext, _: &Yaml) -> BotCmdResult {
    panic!("Panicking for testing purposes....")
}

fn test_stack_overflow(_: HandlerContext, _: &Yaml) -> Reaction {
    //let huge = [[[1usize; 1024]; 1024]; 1024];
    let huge = [[[1usize; 1024]; 1]; 1];
    Reaction::Msg(
        format!(
        "Wow, I allocated {byte_len} bytes on the stack! I have more stack space than I thought.",
        byte_len = mem::size_of_val(&huge) // Ensure that the value is used (theoretically).
    )
        .into(),
    )
}
