# Audio graph

This is the PipeWire graph Helvum shows for the Hamlib dummy, connection 1. Commander starts that radio and creates the two virtual cards. The other boxes were already on the computer. A wire is a PipeWire link. The two halves of one virtual card are joined inside `pw-loopback`, so Helvum draws no wire between them.

The number in each hamspot name is the connection id. The words before it are the make and the model.

The Config tab draws the same wires in the Audio frame. The cartoon radio is the virtual card. The cartoon speaker is the receiver side. The cartoon microphone is the transmit side. Computer devices use the desktop's own icons. Dragging a wire there chooses the same microphone or speaker as the dropdown next to that radio.

## Hamlib Dummy 1 RX

Hamlib Dummy 1 RX is the receiver card. Its PipeWire name is `hamspot_1_rx`. It is a sink: a program plays the receiver here, and the card then carries that audio toward the speakers.

`playback_FL` and `playback_FR` are the left and right inputs. Nothing is wired to them in this picture. A receiver, including a GQRX that Commander starts for this connection, plays into these ports.

`monitor_FL` and `monitor_FR` are a tap of whatever is playing into Hamlib Dummy 1 RX. Nothing is wired to them in this picture.

## hamspot_1_rx

hamspot_1_rx is the speaker side of the same receiver card. Its PipeWire name is `output.hamspot_1_rx`. Audio that enters Hamlib Dummy 1 RX comes out here. Helvum shows the two as separate boxes and does not draw a wire between them.

`output_FL` is the left output. It is wired to USB Audio Device Analog Stereo `playback_FL`.

`output_FR` is the right output. It is wired to USB Audio Device Analog Stereo `playback_FR`.

## USB Audio Device Analog Stereo

USB Audio Device Analog Stereo is the USB sound card's speakers, a C-Media device. It is the computer's default output, so Commander wired the receiver card to it.

`playback_FL` and `playback_FR` are the left and right speaker inputs. They take `output_FL` and `output_FR` from hamspot_1_rx.

`monitor_FL` and `monitor_FR` are a tap of what those speakers are playing. They are wired to the stereo GNOME Settings meter.

## GNOME Settings, stereo

This GNOME Settings box is the Sound panel's peak meter on the speaker path. The subtitle in Helvum is Peak detect. Commander does not create it. The other GNOME Settings box is a second meter, on the microphone path.

`input_FL` listens to USB Audio Device Analog Stereo `monitor_FL`.

`input_FR` listens to USB Audio Device Analog Stereo `monitor_FR`.

`monitor_FL` and `monitor_FR` are this meter's own tap. Nothing is wired to them.

## USB Audio Device Mono

USB Audio Device Mono is the microphone on that same USB sound card. It is the computer's default input, so Commander wired it to the transmit card.

`capture_MONO` is the only microphone output. The transmit card is stereo, so this one port is wired to both hamspot_1_tx `input_FL` and hamspot_1_tx `input_FR`.

## hamspot_1_tx

hamspot_1_tx is the microphone side of the transmit card. Its PipeWire name is `input.hamspot_1_tx`. It collects the computer microphone and feeds Hamlib Dummy 1 TX. Helvum shows no wire between those two boxes.

`input_FL` and `input_FR` both take USB Audio Device Mono `capture_MONO`.

`monitor_FL` and `monitor_FR` are a tap of that microphone audio. Both are wired to the mono GNOME Settings meter.

## Hamlib Dummy 1 TX

Hamlib Dummy 1 TX is the transmit card. Its PipeWire name is `hamspot_1_tx`. It is a source: the radio's microphone. A program that transmits records from this card. A GQRX that Commander starts for this connection opens it as the microphone.

`capture_FL` and `capture_FR` are the left and right outputs a transmitter reads. Nothing is wired to them in this picture. The microphone audio is already inside the card, from hamspot_1_tx.

## GNOME Settings, mono

This GNOME Settings box is the Sound panel's peak meter on the microphone path. The subtitle in Helvum is Peak detect.

`input_MONO` listens to both hamspot_1_tx `monitor_FL` and hamspot_1_tx `monitor_FR`.

`monitor_MONO` is this meter's own tap. Nothing is wired to it.

## Built-in Audio Analog Stereo

Built-in Audio Analog Stereo is the computer's own analog output, the ALC3220 codec on the motherboard. It is not the default speaker, and Commander did not wire the radio to it. It stays on the graph so a wire can be moved here.

`playback_FL` and `playback_FR` are the left and right speaker inputs. Nothing is wired to them.

`monitor_FL` and `monitor_FR` are a tap of those speakers. Nothing is wired to them.

## Wires in this picture

- hamspot_1_rx `output_FL` to USB Audio Device Analog Stereo `playback_FL`
- hamspot_1_rx `output_FR` to USB Audio Device Analog Stereo `playback_FR`
- USB Audio Device Analog Stereo `monitor_FL` to GNOME Settings `input_FL`
- USB Audio Device Analog Stereo `monitor_FR` to GNOME Settings `input_FR`
- USB Audio Device Mono `capture_MONO` to hamspot_1_tx `input_FL`
- USB Audio Device Mono `capture_MONO` to hamspot_1_tx `input_FR`
- hamspot_1_tx `monitor_FL` to GNOME Settings `input_MONO`
- hamspot_1_tx `monitor_FR` to GNOME Settings `input_MONO`
