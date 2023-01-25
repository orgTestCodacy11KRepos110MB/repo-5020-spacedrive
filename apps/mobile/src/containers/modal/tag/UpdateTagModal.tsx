import { forwardRef, useEffect, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import ColorPicker from 'react-native-wheel-color-picker';
import { Tag, queryClient, useLibraryMutation } from '@sd/client';
import { FadeInAnimation } from '~/components/animation/layout';
import { Modal, ModalRef } from '~/components/layout/Modal';
import { Input } from '~/components/primitive/Input';
import useForwardedRef from '~/hooks/useForwardedRef';
import tw from '~/lib/tailwind';

type Props = {
	tag: Tag;
	onSubmit?: () => void;
};

const UpdateTagModal = forwardRef<ModalRef, Props>((props, ref) => {
	const modalRef = useForwardedRef(ref);

	const [tagName, setTagName] = useState(props.tag.name);
	const [tagColor, setTagColor] = useState(props.tag.color);
	const [showPicker, setShowPicker] = useState(false);

	const { mutate: updateTag, isLoading } = useLibraryMutation('tags.update', {
		onSuccess: () => {
			// Reset form
			setShowPicker(false);

			queryClient.invalidateQueries(['tags.list']);

			props.onSubmit?.();
		},
		onSettled: () => {
			// Close dialog
			modalRef.current.dismiss();
		}
	});

	useEffect(() => {
		modalRef.current.snapToIndex(showPicker ? 1 : 0);
	}, [modalRef, showPicker]);

	return (
		<Modal
			ref={modalRef}
			snapPoints={['30%', '60%']}
			onDismiss={() => {
				// Resets form onDismiss
				setShowPicker(false);
			}}
			title="Update Tag"
			// Disable panning gestures
			enableHandlePanningGesture={false}
			enableContentPanningGesture={false}
		>
			<Text style={tw`mb-1 text-xs font-medium text-ink-dull ml-1 mt-3`}>Name</Text>
			<Input value={tagName} onChangeText={(t) => setTagName(t)} />
			<Text style={tw`mb-1 text-xs font-medium text-ink-dull ml-1 mt-3`}>Color</Text>
			<View style={tw`flex flex-row items-center ml-2`}>
				<Pressable
					onPress={() => setShowPicker((v) => !v)}
					style={tw.style({ backgroundColor: tagColor }, 'w-5 h-5 rounded-full')}
				/>
				{/* TODO: Make this editable. Need to make sure color is a valid hexcode and update the color on picker etc. etc. */}
				<Input editable={false} value={tagColor} style={tw`flex-1 ml-2`} />
			</View>
			<FadeInAnimation>
				{showPicker && (
					<View style={tw`h-64 mt-4`}>
						<ColorPicker
							autoResetSlider
							gapSize={0}
							thumbSize={40}
							sliderSize={24}
							shadeSliderThumb
							color={tagColor}
							onColorChangeComplete={(color) => setTagColor(color)}
							swatchesLast={false}
							palette={[
								tw.color('blue-500'),
								tw.color('red-500'),
								tw.color('green-500'),
								tw.color('yellow-500'),
								tw.color('purple-500'),
								tw.color('pink-500'),
								tw.color('gray-500'),
								tw.color('black'),
								tw.color('white')
							]}
						/>
					</View>
				)}
			</FadeInAnimation>
		</Modal>
	);
});

export default UpdateTagModal;